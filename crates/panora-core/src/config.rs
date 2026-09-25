// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! User configuration and platform paths.

use crate::error::{Error, Result};
use directories::ProjectDirs;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Upper bound for `history.max_mime_bytes`.
///
/// A payload reaches the GUI and CLI base64-encoded inside one JSON reply,
/// and clients refuse replies above `ipc::MAX_RESPONSE_BYTES` (64 MiB). At
/// 40 MiB the encoded form stays under 54 MiB, which leaves room for the
/// envelope; a larger limit would let entries be stored that no client can
/// preview or export.
pub const MAX_MIME_BYTES_LIMIT: usize = 40 * 1024 * 1024;

/// Clipboard history limits and retention settings.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HistoryConfig {
    /// Whether PRIMARY selection should be recorded where supported.
    pub record_primary: bool,
    /// Maximum visible, unpinned entries.
    pub max_entries: usize,
    /// Retention period in days; zero means unlimited.
    pub max_age_days: u32,
    /// Maximum bytes read for one MIME payload.
    pub max_mime_bytes: usize,
    /// On Wayland, re-offer the last recorded entry when the clipboard goes
    /// empty because its source application exited: `auto` (only on
    /// compositors that drop the selection, such as Sway or Hyprland;
    /// Mutter and KWin keep it themselves), `always`, or `never`.
    pub persist_on_wayland: String,
    /// Index the text of an entry beyond its 500-character preview (up to
    /// 64 KiB) so search finds words anywhere in it. The index lives in the
    /// same 0600 database file as the previews.
    pub index_full_text: bool,
    /// Bytes of payloads the history may hold. When exceeded, the oldest
    /// unpinned entries go first; pinned entries count but stay. Zero means
    /// no limit.
    pub max_total_bytes: u64,
    /// Image entries kept; the oldest unpinned images go first. Zero means
    /// no limit.
    pub max_images: usize,
    /// What a re-copy of content already at the top of the history does
    /// (CAP-08): `bump` moves it to the top with a fresh timestamp (the
    /// long-standing behaviour), `ignore` leaves the existing entry
    /// exactly where it was. Either way it is still the same entry, never
    /// a duplicate row.
    pub duplicate_policy: String,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            record_primary: false,
            max_entries: 1000,
            max_age_days: 30,
            max_mime_bytes: 10 * 1024 * 1024,
            persist_on_wayland: "auto".into(),
            index_full_text: true,
            max_total_bytes: 512 * 1024 * 1024,
            max_images: 200,
            duplicate_policy: "bump".into(),
        }
    }
}

/// Values `history.duplicate_policy` accepts.
pub const DUPLICATE_POLICIES: &[&str] = &["bump", "ignore"];

/// Privacy-related settings.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PrivacyConfig {
    /// Start with recording paused.
    pub start_private: bool,
    /// Source application names that should never be recorded.
    pub excluded_apps: Vec<String>,
    /// Phrases that keep a copy out of the history while the focused
    /// window's title contains one (case-insensitive; X11 and the GNOME
    /// extension report titles). The title itself is never stored.
    pub excluded_window_titles: Vec<String>,
    /// Text shorter than this many characters (after trimming) is not
    /// recorded; 1 records everything that is not empty.
    pub min_text_length: usize,
    /// Skip text that is nothing but whitespace.
    pub ignore_whitespace_only: bool,
    /// Regular expressions; text matching any of them is not recorded
    /// (for example `"^\\d{16}$"` for bare card numbers). Rust regex syntax.
    pub ignore_patterns: Vec<String>,
    /// Content kinds to record (`text`, `richtext`, `link`, `image`,
    /// `files`, `color`, `binary`). Empty means all of them.
    pub capture_kinds: Vec<String>,
    /// What happens to text that looks like a secret, a key or a card or
    /// account number: `mask` records it with a masked preview and no
    /// full-text index, `drop` never records it, `store` records it like
    /// anything else. Flagged entries expire after `sensitive_ttl_minutes`
    /// under `mask` and `store` alike.
    pub sensitive_policy: String,
    /// Minutes after which a flagged entry is removed; 0 leaves it to the
    /// normal retention rules. Pinned entries stay either way.
    pub sensitive_ttl_minutes: u32,
    /// Minutes of no activity before the daemon engages the second-layer
    /// lock on its own (SEC-02), if a lock password is set; 0 disables
    /// idle locking (the user still locks and unlocks by hand).
    pub lock_after_idle_minutes: u32,
    /// Seconds after a recall before the daemon clears the live system
    /// clipboard on its own (CAP-07), the way a password manager does; 0
    /// disables it. Only fires when the clipboard still holds exactly what
    /// the recall put there — copying something else in the meantime
    /// cancels it. The history entry itself is untouched either way.
    pub clear_clipboard_after_seconds: u32,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            start_private: false,
            // SEC-10 (B-15): the one source for the default list is
            // `privacy::DEFAULT_EXCLUDED_APPS` — `PrivacyEngine::new`
            // always unions it in regardless of what a config.toml says,
            // so a second, independently maintained copy here could drift
            // from what is actually enforced without anyone noticing.
            excluded_apps: crate::privacy::DEFAULT_EXCLUDED_APPS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            excluded_window_titles: Vec::new(),
            min_text_length: 1,
            ignore_whitespace_only: true,
            ignore_patterns: Vec::new(),
            capture_kinds: Vec::new(),
            sensitive_policy: "mask".into(),
            sensitive_ttl_minutes: 10,
            lock_after_idle_minutes: 0,
            clear_clipboard_after_seconds: 0,
        }
    }
}

/// Values `privacy.sensitive_policy` accepts.
pub const SENSITIVE_POLICIES: &[&str] = &["mask", "drop", "store"];

/// Values `ui.position` accepts.
pub const POSITIONS: &[&str] = &["pointer", "center"];

/// Values `ui.layer_anchor` accepts.
pub const LAYER_ANCHORS: &[&str] = &[
    "top-right",
    "top-left",
    "bottom-right",
    "bottom-left",
    "center",
];

/// Upper bounds for the user-defined filters.
pub const MAX_IGNORE_PATTERNS: usize = 32;
/// Longest accepted regular expression, in bytes.
pub const MAX_PATTERN_LEN: usize = 512;
/// Compiled-size cap for one pattern, so a pathological expression cannot
/// eat memory.
pub const PATTERN_SIZE_LIMIT: usize = 1 << 20;

/// Compile one `ignore_patterns` entry the way the daemon will use it: the
/// length and compiled-size limits apply, so a pattern that passes here is
/// accepted by `Config::validate` too. The settings window uses it to refuse
/// a bad pattern before it is saved.
pub fn compile_ignore_pattern(pattern: &str) -> Result<regex::Regex> {
    if pattern.len() > MAX_PATTERN_LEN {
        return Err(Error::Config("ignore_patterns entry too long".into()));
    }
    regex::RegexBuilder::new(pattern)
        .size_limit(PATTERN_SIZE_LIMIT)
        .build()
        .map_err(|e| Error::Config(format!("ignore_patterns: {e}")))
}

/// Content kind names `capture_kinds` accepts.
pub const CONTENT_KIND_NAMES: &[&str] = &[
    "text", "richtext", "link", "image", "files", "color", "binary",
];

/// UI preferences kept intentionally small to avoid runtime state bloat.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct UiConfig {
    /// Interface language; `system`, `tr`, or `en`.
    pub language: String,
    /// Enable automatic paste after recalling an entry where supported.
    pub instant_paste: bool,
    /// Colour scheme; `system`, `light`, or `dark`.
    pub theme: String,
    /// Close the popup when keyboard focus moves to another window, the way
    /// the Windows Win+V flyout does.
    pub close_on_focus_loss: bool,
    /// Where the popup opens: `pointer` (next to the mouse pointer on X11
    /// and, through the Shell extension, on GNOME) or `center`. Wayland
    /// compositors that speak layer-shell use `layer_anchor` instead.
    pub position: String,
    /// Screen corner the popup is anchored to on wlroots compositors
    /// (Sway, Hyprland, ...) when gtk4-layer-shell is installed:
    /// `top-right`, `top-left`, `bottom-right`, `bottom-left` or `center`.
    pub layer_anchor: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: "system".into(),
            instant_paste: false,
            theme: "system".into(),
            close_on_focus_loss: true,
            position: "pointer".into(),
            layer_anchor: "top-right".into(),
        }
    }
}

/// Device sync (SYNC-04). The separate `panora-sync` service reads most of
/// these; `panod` itself only looks at `enabled` and `tombstone_days`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SyncConfig {
    /// Set by `panora-sync` once this device is in a group. While it is on,
    /// deleted entries leave a small record behind for `tombstone_days`,
    /// so other devices learn about the deletion instead of sending the
    /// entry back.
    pub enabled: bool,
    /// Days a deletion is remembered for other devices. A device offline
    /// for longer may bring a deleted entry back.
    pub tombstone_days: u32,
    /// UDP port `panora-sync` listens on; 0 picks a free one (mDNS still
    /// announces it).
    pub port: u16,
    /// Devices to reach directly as `address:port`, for networks that
    /// block mDNS. Only private, link-local and loopback addresses are
    /// ever contacted.
    pub peers: Vec<String>,
    /// Find the other devices with mDNS (`_panora-sync._udp`).
    pub discovery: bool,
    /// Share only pinned entries.
    pub pinned_only: bool,
    /// Share only text-like entries (text, rich text, links, colours).
    pub text_only: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            tombstone_days: 30,
            port: 47_100,
            peers: Vec::new(),
            discovery: true,
            pinned_only: false,
            text_only: false,
        }
    }
}

/// Most `sync.peers` entries accepted.
pub const MAX_SYNC_PEERS: usize = 16;

/// Complete user configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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
    /// Device sync (SYNC-04).
    #[serde(default)]
    pub sync: SyncConfig,
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
        if self.history.max_mime_bytes == 0 || self.history.max_mime_bytes > MAX_MIME_BYTES_LIMIT {
            return Err(Error::Config(format!(
                "max_mime_bytes must be between 1 and {MAX_MIME_BYTES_LIMIT} (40 MiB)"
            )));
        }
        if !matches!(
            self.history.persist_on_wayland.as_str(),
            "auto" | "always" | "never"
        ) {
            return Err(Error::Config(
                "persist_on_wayland must be auto, always or never".into(),
            ));
        }
        if !DUPLICATE_POLICIES.contains(&self.history.duplicate_policy.as_str()) {
            return Err(Error::Config(format!(
                "duplicate_policy must be one of {}",
                DUPLICATE_POLICIES.join(", ")
            )));
        }
        if self.history.max_age_days > 36_500 {
            return Err(Error::Config("max_age_days must be at most 36500".into()));
        }
        if self.privacy.ignore_patterns.len() > MAX_IGNORE_PATTERNS {
            return Err(Error::Config(format!(
                "at most {MAX_IGNORE_PATTERNS} ignore_patterns are allowed"
            )));
        }
        for pattern in &self.privacy.ignore_patterns {
            compile_ignore_pattern(pattern)?;
        }
        if self.privacy.min_text_length > 100_000 {
            return Err(Error::Config("min_text_length is too large".into()));
        }
        if let Some(unknown) = self
            .privacy
            .capture_kinds
            .iter()
            .find(|k| !CONTENT_KIND_NAMES.contains(&k.as_str()))
        {
            return Err(Error::Config(format!(
                "capture_kinds: unknown kind '{unknown}' (expected one of {})",
                CONTENT_KIND_NAMES.join(", ")
            )));
        }
        if !SENSITIVE_POLICIES.contains(&self.privacy.sensitive_policy.as_str()) {
            return Err(Error::Config(format!(
                "sensitive_policy must be one of {}",
                SENSITIVE_POLICIES.join(", ")
            )));
        }
        if self.privacy.sensitive_ttl_minutes > 525_600 {
            return Err(Error::Config(
                "sensitive_ttl_minutes must be at most 525600 (a year)".into(),
            ));
        }
        if self.privacy.lock_after_idle_minutes > 525_600 {
            return Err(Error::Config(
                "lock_after_idle_minutes must be at most 525600 (a year)".into(),
            ));
        }
        if self.privacy.clear_clipboard_after_seconds > 3600 {
            return Err(Error::Config(
                "clear_clipboard_after_seconds must be at most 3600 (an hour)".into(),
            ));
        }
        if !POSITIONS.contains(&self.ui.position.as_str()) {
            return Err(Error::Config(format!(
                "ui.position must be one of {}",
                POSITIONS.join(", ")
            )));
        }
        if !LAYER_ANCHORS.contains(&self.ui.layer_anchor.as_str()) {
            return Err(Error::Config(format!(
                "ui.layer_anchor must be one of {}",
                LAYER_ANCHORS.join(", ")
            )));
        }
        if self.privacy.excluded_window_titles.len() > 64 {
            return Err(Error::Config(
                "at most 64 excluded_window_titles are allowed".into(),
            ));
        }
        if self
            .privacy
            .excluded_window_titles
            .iter()
            .any(|t| t.chars().count() > 256)
        {
            return Err(Error::Config(
                "excluded_window_titles entries must be at most 256 characters".into(),
            ));
        }
        if self.privacy.excluded_apps.len() > 256 {
            return Err(Error::Config("too many excluded applications".into()));
        }
        if self
            .privacy
            .excluded_apps
            .iter()
            .any(|app| app.chars().count() > 256)
        {
            return Err(Error::Config("excluded application name too long".into()));
        }
        if !matches!(self.ui.language.as_str(), "system" | "tr" | "en") {
            return Err(Error::Config("language must be system, tr or en".into()));
        }
        if !matches!(self.ui.theme.as_str(), "system" | "light" | "dark") {
            return Err(Error::Config("theme must be system, light or dark".into()));
        }
        if !(1..=365).contains(&self.sync.tombstone_days) {
            return Err(Error::Config(
                "sync.tombstone_days must be between 1 and 365".into(),
            ));
        }
        if self.sync.peers.len() > MAX_SYNC_PEERS {
            return Err(Error::Config(format!(
                "at most {MAX_SYNC_PEERS} sync.peers are allowed"
            )));
        }
        if let Some(bad) = self
            .sync
            .peers
            .iter()
            .find(|p| p.parse::<std::net::SocketAddr>().is_err())
        {
            return Err(Error::Config(format!(
                "sync.peers: '{bad}' is not an address:port (names are not resolved)"
            )));
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
///
/// The fallback deliberately avoids `/tmp`. That directory is world-writable,
/// so on a multi-user machine without `XDG_RUNTIME_DIR` a local attacker could
/// pre-create `/tmp/panora.sock`: the daemon's `bind` would then fail (the
/// sticky bit stops it from removing a foreign file) and every client would
/// instead connect to the attacker's socket, exposing search terms and letting
/// the attacker feed fabricated entries and image bytes back to the GUI.
/// `data_dir()` is created 0700 and owned by the user, so it cannot be squatted.
pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(data_dir)
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
        assert_eq!(cfg.ui.theme, "system");
    }

    /// SEC-10 (B-15): the default exclusion list used to be duplicated
    /// here and in `privacy::DEFAULT_EXCLUDED_APPS`, with different
    /// contents in each copy. Now `PrivacyConfig::default` is built
    /// straight from the one in `privacy`; this pins that down so a
    /// hardcoded list cannot quietly come back and drift from it again.
    #[test]
    fn excluded_apps_default_has_one_source() {
        let expected: Vec<String> = crate::privacy::DEFAULT_EXCLUDED_APPS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(PrivacyConfig::default().excluded_apps, expected);
    }

    #[test]
    fn sync_section_defaults_and_limits() {
        let cfg: Config = toml::from_str("[sync]\nenabled = true\n").unwrap();
        cfg.validate().unwrap();
        assert!(cfg.sync.enabled);
        assert_eq!(cfg.sync.tombstone_days, 30);
        assert!(cfg.sync.discovery);

        let mut cfg = Config::default();
        cfg.sync.peers = vec!["192.168.1.20:47100".into(), "[fe80::1]:47100".into()];
        cfg.validate().unwrap();
        cfg.sync.peers.push("laptop.local:47100".into());
        assert!(cfg.validate().is_err());
        let mut cfg = Config::default();
        cfg.sync.tombstone_days = 0;
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let cfg: Config = toml::from_str("[history]\nmax_entries = 5\n").unwrap();
        cfg.validate().unwrap();
        assert_eq!(cfg.history.max_entries, 5);
        assert_eq!(cfg.history.max_age_days, 30);
        assert_eq!(cfg.ui.language, "system");
    }

    #[test]
    fn invalid_values_are_rejected() {
        let mut cfg = Config::default();
        cfg.ui.language = "klingon".into();
        assert!(cfg.validate().is_err());
        let mut cfg = Config::default();
        cfg.history.max_entries = 0;
        assert!(cfg.validate().is_err());
        let mut cfg = Config::default();
        cfg.ui.theme = "sepia".into();
        assert!(cfg.validate().is_err());
        // Anything the IPC reply cap could not carry is rejected up front.
        let mut cfg = Config::default();
        cfg.history.max_mime_bytes = MAX_MIME_BYTES_LIMIT;
        cfg.validate().unwrap();
        cfg.history.max_mime_bytes = MAX_MIME_BYTES_LIMIT + 1;
        assert!(cfg.validate().is_err());
        // Filters: a bad regex, an unknown kind and an absurd length fail.
        let mut cfg = Config::default();
        cfg.privacy.ignore_patterns = vec!["^\\d{16}$".into(), "(unclosed".into()];
        assert!(cfg.validate().is_err());
        let mut cfg = Config::default();
        cfg.privacy.capture_kinds = vec!["text".into(), "movie".into()];
        assert!(cfg.validate().is_err());
        let mut cfg = Config::default();
        cfg.privacy.ignore_patterns = vec!["^\\d{16}$".into()];
        cfg.privacy.capture_kinds = vec!["text".into(), "link".into()];
        cfg.privacy.min_text_length = 3;
        cfg.validate().unwrap();
        // CAP-08 / CAP-07.
        let mut cfg = Config::default();
        cfg.history.duplicate_policy = "always".into();
        assert!(cfg.validate().is_err());
        cfg.history.duplicate_policy = "ignore".into();
        cfg.validate().unwrap();
        let mut cfg = Config::default();
        cfg.privacy.clear_clipboard_after_seconds = 3601;
        assert!(cfg.validate().is_err());
        cfg.privacy.clear_clipboard_after_seconds = 3600;
        cfg.validate().unwrap();
    }

    #[test]
    fn roundtrips_through_toml() {
        let mut cfg = Config::default();
        cfg.privacy.excluded_apps.push("mybank".into());
        cfg.ui.instant_paste = true;
        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert!(back.ui.instant_paste);
        assert!(back.privacy.excluded_apps.contains(&"mybank".to_string()));
    }
}
