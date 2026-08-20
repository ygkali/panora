// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! User configuration and platform paths.

use crate::error::{Error, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Clipboard history limits and retention settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    /// Whether PRIMARY selection should be recorded where supported.
    pub record_primary: bool,
    /// Maximum visible, unpinned entries.
    pub max_entries: usize,
    /// Retention period in days; zero means unlimited.
    pub max_age_days: u32,
    /// Maximum bytes read for one MIME payload.
    pub max_mime_bytes: usize,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            record_primary: false,
            max_entries: 1000,
            max_age_days: 30,
            max_mime_bytes: 10 * 1024 * 1024,
        }
    }
}

/// Privacy-related settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyConfig {
    /// Start with recording paused.
    pub start_private: bool,
    /// Source application names that should never be recorded.
    pub excluded_apps: Vec<String>,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            start_private: false,
            excluded_apps: vec![
                "keepassxc".into(),
                "bitwarden".into(),
                "1password".into(),
                "gnome-secrets".into(),
            ],
        }
    }
}

/// UI preferences kept intentionally small to avoid runtime state bloat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    /// Interface language; `system`, `tr`, or `en`.
    pub language: String,
    /// Enable automatic paste after recalling an entry where supported.
    pub instant_paste: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: "system".into(),
            instant_paste: false,
        }
    }
}

/// Complete user configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// History and payload limits.
    #[serde(default)]
    pub history: HistoryConfig,
    /// Privacy policy.
    #[serde(default)]
    pub privacy: PrivacyConfig,
    /// UI preferences.
    #[serde(default)]
    pub ui: UiConfig,
}

impl Config {
    /// Load configuration, using defaults when the file does not exist.
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&text)?;
        config.validate()?;
        Ok(config)
    }

    /// Validate safety limits before use.
    pub fn validate(&self) -> Result<()> {
        if self.history.max_entries == 0 || self.history.max_entries > 100_000 {
            return Err(Error::Config(
                "max_entries must be between 1 and 100000".into(),
            ));
        }
        if self.history.max_mime_bytes == 0 || self.history.max_mime_bytes > 256 * 1024 * 1024 {
            return Err(Error::Config(
                "max_mime_bytes is outside the safe range".into(),
            ));
        }
        if self.privacy.excluded_apps.len() > 256 {
            return Err(Error::Config("too many excluded applications".into()));
        }
        Ok(())
    }

    /// Persist configuration atomically with private permissions.
    pub fn save(&self) -> Result<()> {
        self.validate()?;
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            set_mode(parent, 0o700)?;
        }
        let text = toml::to_string_pretty(self)?;
        let temp = path.with_extension("toml.tmp");
        std::fs::write(&temp, text)?;
        set_mode(&temp, 0o600)?;
        std::fs::rename(temp, path)?;
        Ok(())
    }
}

/// Panora configuration file path.
pub fn config_path() -> PathBuf {
    ProjectDirs::from("org", "Panora", "panora")
        .map(|d| d.config_dir().join("config.toml"))
        .unwrap_or_else(|| PathBuf::from(".config/panora/config.toml"))
}

/// Panora data directory path.
pub fn data_dir() -> PathBuf {
    ProjectDirs::from("org", "Panora", "panora")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".local/share/panora"))
}

/// Panora user IPC socket path.
pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("panora.sock")
}

fn set_mode(path: &std::path::Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_safe() {
        let cfg = Config::default();
        cfg.validate().unwrap();
        assert_eq!(cfg.history.max_mime_bytes, 10 * 1024 * 1024);
        assert!(!cfg.privacy.excluded_apps.is_empty());
    }
}
