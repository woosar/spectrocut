use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::{
        event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Paragraph, Wrap},
};

use super::field::{Field, Kind};
use super::file_picker::FilePicker;
use super::preset_store::PresetStore;
use super::render_job::RenderJob;
use super::settings::Settings;
use super::tool_manager::ToolManager;
use crate::helpers::{
    centered_line_box, centered_rect, draw_input, fmt_duration, one_line_command, pretty_command,
    probe_duration,
};

enum Mode {
    Form,
    Edit {
        buffer: String,
        error: Option<String>,
    },
    Picker(FilePicker),
    SavePreset {
        buffer: String,
    },
    LoadPreset {
        names: Vec<String>,
        state: ListState,
    },
    ConfirmOverwrite,
    Tools,
    Rendering {
        job: RenderJob,
        confirm_cancel: bool,
    },
    Done {
        job: RenderJob,
    },
}

pub struct App {
    settings: Settings,
    fields: Vec<Field>,
    selected: usize,
    scroll: usize,
    mode: Mode,
    presets: PresetStore,
    tools: ToolManager,
    status: String,
    duration_cache: Option<(String, Option<f64>)>,
    run_package_install: bool,
    quit_with: Option<Option<String>>,
}

impl App {
    pub fn new() -> Self {
        let presets = PresetStore::new();
        let settings = presets.load_last().unwrap_or_default();
        Self {
            settings,
            fields: Field::all(),
            selected: 0,
            scroll: 0,
            mode: Mode::Form,
            presets,
            tools: ToolManager::new(),
            status: String::new(),
            duration_cache: None,
            run_package_install: false,
            quit_with: None,
        }
    }

    /// Main loop. Returns a command line to print after the terminal is restored, if requested.
    pub fn run(mut self, terminal: &mut DefaultTerminal) -> Result<Option<String>> {
        if !self.tools.all_ok() {
            self.mode = Mode::Tools;
            self.tools.message = "ffmpeg is missing or incomplete; install it here first.".into();
            self.tools.check_latest();
        }
        loop {
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.on_key(key);
            }
            self.tick();
            if self.run_package_install {
                self.run_package_install = false;
                self.package_install(terminal)?;
            }
            if let Some(result) = self.quit_with.take() {
                self.presets.save_last(&self.settings);
                return Ok(result);
            }
        }
    }

    fn tick(&mut self) {
        self.tools.poll();
        if let Mode::Rendering { job, .. } = &mut self.mode {
            job.poll();
            if job.finished.is_some()
                && let Mode::Rendering { job, .. } = std::mem::replace(&mut self.mode, Mode::Form)
            {
                self.mode = Mode::Done { job };
            }
        }
    }

    fn audio_duration(&mut self) -> Option<f64> {
        let audio = self.settings.audio.clone();
        match &self.duration_cache {
            Some((path, d)) if *path == audio => *d,
            _ => {
                let d = probe_duration(&audio);
                self.duration_cache = Some((audio, d));
                d
            }
        }
    }

    // ---------------------------------------------------------------- input

    fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            if let Mode::Rendering { job, .. } = &mut self.mode {
                job.kill();
            }
            self.quit_with = Some(None);
            return;
        }
        let mode = std::mem::replace(&mut self.mode, Mode::Form);
        self.mode = match mode {
            Mode::Form => self.on_form_key(key),
            Mode::Edit { buffer, error } => self.on_edit_key(key, buffer, error),
            Mode::Picker(picker) => self.on_picker_key(key, picker),
            Mode::SavePreset { buffer } => self.on_save_key(key, buffer),
            Mode::LoadPreset { names, state } => self.on_load_key(key, names, state),
            Mode::ConfirmOverwrite => match key.code {
                KeyCode::Char('y' | 'Y') => self.start_render(),
                _ => {
                    self.status = "Render cancelled.".into();
                    Mode::Form
                }
            },
            Mode::Tools => self.on_tools_key(key),
            Mode::Rendering {
                mut job,
                confirm_cancel,
            } => match (confirm_cancel, key.code) {
                (false, KeyCode::Esc | KeyCode::Char('q')) => Mode::Rendering {
                    job,
                    confirm_cancel: true,
                },
                (true, KeyCode::Char('y' | 'Y')) => {
                    job.kill();
                    Mode::Done { job }
                }
                (true, _) => Mode::Rendering {
                    job,
                    confirm_cancel: false,
                },
                (false, _) => Mode::Rendering {
                    job,
                    confirm_cancel,
                },
            },
            Mode::Done { job } => match key.code {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q' | ' ') => Mode::Form,
                _ => Mode::Done { job },
            },
        };
    }

    fn on_form_key(&mut self, key: KeyEvent) -> Mode {
        let last = self.fields.len() - 1;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.selected = (self.selected + 1).min(last),
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(8),
            KeyCode::PageDown => self.selected = (self.selected + 8).min(last),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = last,
            KeyCode::Left => self.fields[self.selected].adjust(&mut self.settings, -1),
            KeyCode::Right => self.fields[self.selected].adjust(&mut self.settings, 1),
            KeyCode::Enter | KeyCode::Char(' ') => {
                let field = &self.fields[self.selected];
                match field.kind {
                    Kind::Path(kind) => {
                        return Mode::Picker(FilePicker::new(
                            &field.edit_text(&self.settings),
                            kind.extensions(),
                        ));
                    }
                    Kind::Choice(_) | Kind::Toggle => field.adjust(&mut self.settings, 1),
                    _ => {
                        return Mode::Edit {
                            buffer: field.edit_text(&self.settings),
                            error: None,
                        };
                    }
                }
            }
            KeyCode::Char('e') if self.fields[self.selected].is_typeable() => {
                let buffer = self.fields[self.selected].edit_text(&self.settings);
                return Mode::Edit {
                    buffer,
                    error: None,
                };
            }
            KeyCode::Char('r') => return self.request_render(),
            KeyCode::Char('s') => {
                return Mode::SavePreset {
                    buffer: String::new(),
                };
            }
            KeyCode::Char('l') => {
                let names = self.presets.list();
                if names.is_empty() {
                    self.status = "No presets saved yet (press s to save one).".into();
                } else {
                    let mut state = ListState::default();
                    state.select(Some(0));
                    return Mode::LoadPreset { names, state };
                }
            }
            KeyCode::Char('D') => {
                let keep = (
                    self.settings.image.clone(),
                    self.settings.audio.clone(),
                    self.settings.output.clone(),
                );
                self.settings = Settings::default();
                (
                    self.settings.image,
                    self.settings.audio,
                    self.settings.output,
                ) = keep;
                self.status = "All options reset to defaults (files kept).".into();
            }
            KeyCode::Char('t') => {
                self.tools.refresh();
                self.tools.check_latest();
                return Mode::Tools;
            }
            KeyCode::Char('c') => self.quit_with = Some(Some(one_line_command(&self.settings))),
            KeyCode::Char('q') | KeyCode::Esc => self.quit_with = Some(None),
            _ => {}
        }
        Mode::Form
    }

    fn on_edit_key(&mut self, key: KeyEvent, mut buffer: String, error: Option<String>) -> Mode {
        match key.code {
            KeyCode::Esc => return Mode::Form,
            KeyCode::Enter => {
                return match self.fields[self.selected].commit(&mut self.settings, &buffer) {
                    Ok(()) => Mode::Form,
                    Err(e) => Mode::Edit {
                        buffer,
                        error: Some(e),
                    },
                };
            }
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(c) => buffer.push(c),
            _ => return Mode::Edit { buffer, error },
        }
        Mode::Edit {
            buffer,
            error: None,
        }
    }

    fn on_picker_key(&mut self, key: KeyEvent, mut picker: FilePicker) -> Mode {
        match key.code {
            KeyCode::Esc => return Mode::Form,
            KeyCode::Up | KeyCode::Char('k') => picker.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => picker.move_by(1),
            KeyCode::PageUp => picker.move_by(-10),
            KeyCode::PageDown => picker.move_by(10),
            KeyCode::Backspace | KeyCode::Left => picker.go_up(),
            KeyCode::Char('.') => picker.toggle_hidden(),
            KeyCode::Enter | KeyCode::Right => {
                if let Some(path) = picker.activate() {
                    let _ = self.fields[self.selected].commit(&mut self.settings, &path);
                    return Mode::Form;
                }
            }
            _ => {}
        }
        Mode::Picker(picker)
    }

    fn on_save_key(&mut self, key: KeyEvent, mut buffer: String) -> Mode {
        match key.code {
            KeyCode::Esc => return Mode::Form,
            KeyCode::Enter => {
                self.status = match self.presets.save(&buffer, &self.settings) {
                    Ok(path) => format!("Saved preset to {}", path.display()),
                    Err(e) => format!("Could not save preset: {e}"),
                };
                return Mode::Form;
            }
            KeyCode::Backspace => {
                buffer.pop();
            }
            KeyCode::Char(c) => buffer.push(c),
            _ => {}
        }
        Mode::SavePreset { buffer }
    }

    fn on_load_key(&mut self, key: KeyEvent, names: Vec<String>, mut state: ListState) -> Mode {
        let cur = state.selected().unwrap_or(0);
        match key.code {
            KeyCode::Esc => return Mode::Form,
            KeyCode::Up | KeyCode::Char('k') => state.select(Some(cur.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => {
                state.select(Some((cur + 1).min(names.len() - 1)))
            }
            KeyCode::Enter => {
                let name = &names[cur];
                self.status = match self.presets.load(name) {
                    Ok(loaded) => {
                        // Presets carry the look; keep the current files unless the preset has some.
                        let keep = self.settings.clone();
                        self.settings = loaded;
                        for (dst, src) in [
                            (&mut self.settings.image, keep.image),
                            (&mut self.settings.audio, keep.audio),
                        ] {
                            if dst.is_empty() {
                                *dst = src;
                            }
                        }
                        format!("Loaded preset '{name}'.")
                    }
                    Err(e) => format!("Could not load '{name}': {e}"),
                };
                return Mode::Form;
            }
            _ => {}
        }
        Mode::LoadPreset { names, state }
    }

    fn on_tools_key(&mut self, key: KeyEvent) -> Mode {
        match key.code {
            KeyCode::Char('d') => self.tools.start_download(),
            KeyCode::Char('u') => self.tools.check_latest(),
            KeyCode::Char('r') => {
                self.tools.refresh();
                self.tools.message = "Rescanned PATH.".into();
            }
            KeyCode::Char('a') => self.tools.add_to_shell_path(),
            KeyCode::Char('p') if !self.tools.is_busy() => {
                if self.tools.package_command().is_some() {
                    self.run_package_install = true;
                }
            }
            KeyCode::Esc | KeyCode::Char('q' | 't') => return Mode::Form,
            _ => {}
        }
        Mode::Tools
    }

    // ---------------------------------------------------------------- actions

    fn request_render(&mut self) -> Mode {
        if !self.tools.all_ok() {
            self.tools.message =
                "Rendering needs ffmpeg + ffprobe with libx264. Install them here.".into();
            self.tools.check_latest();
            return Mode::Tools;
        }
        let s = &self.settings;
        let problem = if !Path::new(&s.image).is_file() {
            Some("Pick an existing image first.")
        } else if !Path::new(&s.audio).is_file() {
            Some("Pick an existing audio file first.")
        } else if s.output.trim().is_empty() {
            Some("Set an output file name.")
        } else if s.width % 2 != 0 || s.height % 2 != 0 {
            Some("Width and height must be even numbers for yuv420p.")
        } else if s.output == s.image || s.output == s.audio {
            Some("Output would overwrite an input file!")
        } else {
            None
        };
        if let Some(p) = problem {
            self.status = p.into();
            return Mode::Form;
        }
        if Path::new(&s.output).exists() {
            return Mode::ConfirmOverwrite;
        }
        self.start_render()
    }

    fn start_render(&mut self) -> Mode {
        let total = self.audio_duration();
        match RenderJob::spawn(&self.settings, total) {
            Ok(job) => {
                self.presets.save_last(&self.settings);
                Mode::Rendering {
                    job,
                    confirm_cancel: false,
                }
            }
            Err(e) => {
                self.status = format!("{e:#}");
                Mode::Form
            }
        }
    }

    /// Leave the TUI, run the package manager interactively (sudo may prompt), then come back.
    fn package_install(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let Some(cmd) = self.tools.package_command() else {
            return Ok(());
        };
        disable_raw_mode()?;
        execute!(std::io::stdout(), LeaveAlternateScreen)?;
        println!("$ {}\n", cmd.display());
        let result = Command::new(&cmd.program).args(&cmd.args).status();
        println!("\nPress Enter to return to spectrocut…");
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnterAlternateScreen)?;
        terminal.clear()?;
        self.tools.refresh();
        self.tools.message = match result {
            Ok(st) if st.success() && self.tools.all_ok() => "Package install finished, ffmpeg is ready.".into(),
            Ok(st) if st.success() => {
                "Package install finished, but ffmpeg is not visible yet. Restart spectrocut (PATH may have changed).".into()
            }
            Ok(st) => format!("Package manager exited with {st}."),
            Err(e) => format!("Could not run {}: {e}", cmd.program),
        };
        Ok(())
    }

    // ---------------------------------------------------------------- drawing

    fn draw(&mut self, frame: &mut Frame) {
        let [title, body, status, keys] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

        let tab = |name: &str, active: bool| {
            if active {
                Span::styled(
                    format!(" {name} "),
                    Style::default().fg(Color::Black).bg(Color::Green),
                )
            } else {
                Span::styled(format!(" {name} "), Style::default().fg(Color::DarkGray))
            }
        };
        let on_tools = matches!(self.mode, Mode::Tools);
        let rendering = matches!(self.mode, Mode::Rendering { .. } | Mode::Done { .. });
        frame.render_widget(
            Line::from(vec![
                Span::styled(
                    " spectrocut ",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("image + audio spectrum → mp4   "),
                tab("Settings", !on_tools && !rendering),
                tab("Tools [t]", on_tools),
                tab("Render", rendering),
            ]),
            title,
        );

        match &self.mode {
            Mode::Tools => self.tools.draw(frame, body),
            Mode::Rendering { .. } | Mode::Done { .. } => self.draw_render(frame, body),
            _ => self.draw_form(frame, body),
        }

        // Popups over the form.
        match &mut self.mode {
            Mode::Edit { buffer, error } => {
                let label = self.fields[self.selected].label;
                let title = match error {
                    Some(e) => format!(" {label}: {e} "),
                    None => format!(" {label} (Enter to apply, Esc to cancel) "),
                };
                draw_input(frame, body, &title, buffer, error.is_some());
            }
            Mode::SavePreset { buffer } => {
                draw_input(
                    frame,
                    body,
                    " Save preset as (Enter to save, Esc to cancel) ",
                    buffer,
                    false,
                );
            }
            Mode::Picker(picker) => {
                let label = self.fields[self.selected].label;
                picker.draw(frame, centered_rect(body, 80, 80), label);
            }
            Mode::LoadPreset { names, state } => {
                let area = centered_rect(body, 50, 60);
                frame.render_widget(Clear, area);
                let items: Vec<ListItem> =
                    names.iter().map(|n| ListItem::new(n.as_str())).collect();
                let list = List::new(items)
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(" Load preset (Enter/Esc) "),
                    )
                    .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
                    .highlight_symbol("▶ ");
                frame.render_stateful_widget(list, area, state);
            }
            Mode::ConfirmOverwrite => {
                let area = centered_line_box(body, 60, 3);
                frame.render_widget(Clear, area);
                frame.render_widget(
                    Paragraph::new(format!("{} exists. Overwrite? (y/n)", self.settings.output))
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .border_style(Style::default().fg(Color::Yellow)),
                        ),
                    area,
                );
            }
            _ => {}
        }

        frame.render_widget(
            Line::styled(self.status.clone(), Style::default().fg(Color::Yellow)),
            status,
        );
        let hints = match &self.mode {
            Mode::Form => {
                "↑↓ move  ←→ change  Enter edit/pick  r render  s save  l load  D defaults  t tools  c print cmd  q quit"
            }
            Mode::Edit { .. } | Mode::SavePreset { .. } => {
                "type  Backspace delete  Enter apply  Esc cancel"
            }
            Mode::Picker(_) => {
                "↑↓ move  Enter open/select  Backspace up  . hidden files  Esc cancel"
            }
            Mode::LoadPreset { .. } => "↑↓ move  Enter load  Esc cancel",
            Mode::ConfirmOverwrite => "y overwrite  any other key cancels",
            Mode::Tools => {
                "d download/update  p package manager  a add to PATH  u check  r rescan  Esc back"
            }
            Mode::Rendering {
                confirm_cancel: true,
                ..
            } => "Cancel render? y = yes, any other key = keep going",
            Mode::Rendering { .. } => "Esc cancel render",
            Mode::Done { .. } => "Enter back to settings",
        };
        frame.render_widget(
            Line::styled(hints, Style::default().fg(Color::DarkGray)),
            keys,
        );
    }

    fn draw_form(&mut self, frame: &mut Frame, area: Rect) {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(48), Constraint::Percentage(52)])
                .areas(area);

        // Field list with group headers.
        let mut lines: Vec<Line> = Vec::new();
        let mut selected_line = 0;
        let mut group = "";
        for (i, f) in self.fields.iter().enumerate() {
            if f.group != group {
                group = f.group;
                if !lines.is_empty() {
                    lines.push(Line::raw(""));
                }
                lines.push(Line::styled(
                    format!(" {group}"),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            let value = f.display(&self.settings);
            let is_sel = i == self.selected;
            if is_sel {
                selected_line = lines.len();
            }
            let value = match (&f.kind, is_sel) {
                (Kind::Choice(_) | Kind::Int { .. } | Kind::Float { .. } | Kind::Toggle, true) => {
                    format!("◀ {value} ▶")
                }
                _ => value,
            };
            let value_style = match &f.kind {
                Kind::Toggle if value.contains("on") => Style::default().fg(Color::Green),
                Kind::Toggle => Style::default().fg(Color::DarkGray),
                Kind::Path(_) if !Path::new(&f.edit_text(&self.settings)).is_file() => {
                    Style::default().fg(Color::Red)
                }
                _ => Style::default().fg(Color::White),
            };
            let mut line = Line::from(vec![
                Span::raw(if is_sel { " ▶ " } else { "   " }),
                Span::raw(format!("{:<17}", f.label)),
                Span::styled(value, value_style),
            ]);
            if is_sel {
                line = line.style(
                    Style::default()
                        .bg(Color::Rgb(40, 40, 60))
                        .add_modifier(Modifier::BOLD),
                );
            }
            lines.push(line);
        }
        let inner_h = left.height.saturating_sub(2) as usize;
        let top = selected_line.saturating_sub(1); // keep the group header visible
        if top < self.scroll {
            self.scroll = top;
        } else if selected_line >= self.scroll + inner_h {
            self.scroll = selected_line + 1 - inner_h;
        }
        frame.render_widget(
            Paragraph::new(lines)
                .scroll((self.scroll as u16, 0))
                .block(Block::default().borders(Borders::ALL).title(" Settings ")),
            left,
        );

        // Command preview + info.
        let [cmd_area, info_area] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(7)]).areas(right);
        frame.render_widget(
            Paragraph::new(pretty_command(&self.settings))
                .style(Style::default().fg(Color::Gray))
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Command (c prints it on exit) "),
                ),
            cmd_area,
        );
        let duration = self.audio_duration().map_or("-".to_string(), fmt_duration);
        let field = &self.fields[self.selected];
        let info = vec![
            Line::from(vec![
                Span::styled(
                    format!("{}: ", field.label),
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(field.help),
            ]),
            Line::raw(""),
            Line::from(vec![
                Span::styled("Audio length: ", Style::default().fg(Color::Cyan)),
                Span::raw(duration),
            ]),
            Line::from(vec![
                Span::styled("Output: ", Style::default().fg(Color::Cyan)),
                Span::raw(format!(
                    "{}x{} @ {} fps, CRF {}, {}",
                    self.settings.width,
                    self.settings.height,
                    self.settings.fps,
                    self.settings.crf,
                    self.settings.preset
                )),
            ]),
        ];
        frame.render_widget(
            Paragraph::new(info)
                .wrap(Wrap { trim: true })
                .block(Block::default().borders(Borders::ALL).title(" Info ")),
            info_area,
        );
    }

    fn draw_render(&self, frame: &mut Frame, area: Rect) {
        let (job, done) = match &self.mode {
            Mode::Rendering { job, .. } => (job, false),
            Mode::Done { job } => (job, true),
            _ => return,
        };
        let [gauge_area, stats_area, log_area] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(4),
            Constraint::Min(3),
        ])
        .areas(area);

        let (color, title) = match (&job.finished, done) {
            (Some(Ok(())), _) => (Color::Green, " Finished ".to_string()),
            (Some(Err(e)), _) => (Color::Red, format!(" Failed: {e} ")),
            _ => (
                Color::Green,
                format!(" Rendering {} ", self.settings.output),
            ),
        };
        let ratio = if matches!(job.finished, Some(Ok(()))) {
            1.0
        } else {
            job.ratio()
        };
        frame.render_widget(
            Gauge::default()
                .block(Block::default().borders(Borders::ALL).title(title))
                .gauge_style(Style::default().fg(color))
                .ratio(ratio)
                .label(format!("{:.1}%", ratio * 100.0)),
            gauge_area,
        );

        let total = job.total_secs.map_or("?".into(), fmt_duration);
        let eta = job.eta_secs().map_or("-".into(), fmt_duration);
        let stats = vec![
            Line::raw(format!(
                "Position {} / {}    Elapsed {}    ETA {}",
                fmt_duration(job.out_secs),
                total,
                fmt_duration(job.elapsed_secs()),
                eta
            )),
            Line::raw(format!("Encoding at {} fps, speed {}", job.fps, job.speed)),
        ];
        frame.render_widget(
            Paragraph::new(stats).block(Block::default().borders(Borders::ALL)),
            stats_area,
        );

        let h = log_area.height.saturating_sub(2) as usize;
        let lines: Vec<Line> = job
            .log
            .iter()
            .skip(job.log.len().saturating_sub(h))
            .map(|l| Line::raw(l.as_str()))
            .collect();
        frame.render_widget(
            Paragraph::new(lines)
                .style(Style::default().fg(Color::DarkGray))
                .block(Block::default().borders(Borders::ALL).title(" ffmpeg log ")),
            log_area,
        );
    }
}
