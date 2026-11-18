use std::path::{Path, PathBuf};

use crate::helpers::display_path;

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, List, ListItem, ListState},
};

struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

/// A small directory browser that only shows folders and files with matching extensions.
pub struct FilePicker {
    dir: PathBuf,
    entries: Vec<Entry>,
    state: ListState,
    extensions: &'static [&'static str],
    show_hidden: bool,
}

impl FilePicker {
    pub fn new(current_value: &str, extensions: &'static [&'static str]) -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let start = Path::new(current_value)
            .parent()
            .map(|p| {
                if p.as_os_str().is_empty() {
                    cwd.clone()
                } else {
                    cwd.join(p)
                }
            })
            .filter(|p| p.is_dir())
            .unwrap_or(cwd);
        let mut picker = Self {
            dir: start,
            entries: Vec::new(),
            state: ListState::default(),
            extensions,
            show_hidden: false,
        };
        picker.reload();
        picker
    }

    fn reload(&mut self) {
        self.dir = self.dir.canonicalize().unwrap_or_else(|_| self.dir.clone());
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        if let Ok(read) = std::fs::read_dir(&self.dir) {
            for item in read.flatten() {
                let path = item.path();
                let name = item.file_name().to_string_lossy().to_string();
                if !self.show_hidden && name.starts_with('.') {
                    continue;
                }
                if path.is_dir() {
                    dirs.push(Entry {
                        name: format!("{name}/"),
                        path,
                        is_dir: true,
                    });
                } else {
                    let ext = path
                        .extension()
                        .map(|e| e.to_string_lossy().to_lowercase())
                        .unwrap_or_default();
                    if self.extensions.contains(&ext.as_str()) {
                        files.push(Entry {
                            name,
                            path,
                            is_dir: false,
                        });
                    }
                }
            }
        }
        dirs.sort_by_key(|e| e.name.to_lowercase());
        files.sort_by_key(|e| e.name.to_lowercase());
        self.entries.clear();
        if let Some(parent) = self.dir.parent() {
            self.entries.push(Entry {
                name: "../".into(),
                path: parent.to_path_buf(),
                is_dir: true,
            });
        }
        self.entries.extend(dirs);
        self.entries.extend(files);
        self.state.select(if self.entries.is_empty() {
            None
        } else {
            Some(0)
        });
    }

    pub fn move_by(&mut self, delta: i64) {
        if self.entries.is_empty() {
            return;
        }
        let max = self.entries.len() as i64 - 1;
        let cur = self.state.selected().unwrap_or(0) as i64;
        self.state
            .select(Some((cur + delta).clamp(0, max) as usize));
    }

    pub fn go_up(&mut self) {
        if let Some(parent) = self.dir.parent() {
            self.dir = parent.to_path_buf();
            self.reload();
        }
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.reload();
    }

    /// Enter on the selection: descends into folders, returns the chosen file.
    /// Paths below the working directory are returned relative to it.
    pub fn activate(&mut self) -> Option<String> {
        let entry = self.entries.get(self.state.selected()?)?;
        if entry.is_dir {
            self.dir = entry.path.clone();
            self.reload();
            return None;
        }
        let cwd = std::env::current_dir()
            .ok()
            .and_then(|c| c.canonicalize().ok());
        let chosen = entry
            .path
            .canonicalize()
            .unwrap_or_else(|_| entry.path.clone());
        let shown = cwd
            .and_then(|c| chosen.strip_prefix(&c).ok().map(Path::to_path_buf))
            .unwrap_or(chosen);
        Some(display_path(&shown))
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, title: &str) {
        frame.render_widget(Clear, area);
        let items: Vec<ListItem> = self
            .entries
            .iter()
            .map(|e| {
                let style = if e.is_dir {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                };
                ListItem::new(Line::styled(e.name.clone(), style))
            })
            .collect();
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {title}: {} ", display_path(&self.dir)))
            .title_bottom(" Enter open/select · Backspace up · . hidden · Esc cancel ");
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, area, &mut self.state);
    }
}
