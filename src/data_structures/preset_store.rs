use std::path::PathBuf;

use anyhow::{Context, Result};

use super::settings::Settings;

/// TOML presets in `~/.config/spectrocut/presets/`, plus the last-used settings.
pub struct PresetStore {
    config_dir: Option<PathBuf>,
}

impl PresetStore {
    pub fn new() -> Self {
        let config_dir = directories::ProjectDirs::from("", "", "spectrocut")
            .map(|d| d.config_dir().to_path_buf());
        Self { config_dir }
    }

    fn presets_dir(&self) -> Result<PathBuf> {
        let dir = self
            .config_dir
            .as_ref()
            .context("no config directory available")?
            .join("presets");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub fn list(&self) -> Vec<String> {
        let Ok(dir) = self.presets_dir() else {
            return Vec::new();
        };
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.extension()? != "toml" {
                    return None;
                }
                Some(p.file_stem()?.to_string_lossy().to_string())
            })
            .collect();
        names.sort();
        names
    }

    pub fn save(&self, name: &str, settings: &Settings) -> Result<PathBuf> {
        let safe: String = name
            .trim()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || "-_ ".contains(c) {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        anyhow::ensure!(!safe.is_empty(), "preset name is empty");
        let path = self.presets_dir()?.join(format!("{safe}.toml"));
        std::fs::write(&path, toml::to_string_pretty(settings)?)?;
        Ok(path)
    }

    pub fn load(&self, name: &str) -> Result<Settings> {
        let path = self.presets_dir()?.join(format!("{name}.toml"));
        let text = std::fs::read_to_string(&path)?;
        Ok(toml::from_str(&text)?)
    }

    fn last_path(&self) -> Option<PathBuf> {
        self.config_dir.as_ref().map(|d| d.join("last.toml"))
    }

    pub fn load_last(&self) -> Option<Settings> {
        let text = std::fs::read_to_string(self.last_path()?).ok()?;
        toml::from_str(&text).ok()
    }

    pub fn save_last(&self, settings: &Settings) {
        if let (Some(path), Ok(text)) = (self.last_path(), toml::to_string_pretty(settings)) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, text);
        }
    }
}
