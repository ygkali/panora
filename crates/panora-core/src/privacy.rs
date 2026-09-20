// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Privacy engine: decides whether captured clipboard data may be stored.
//!
//! Three independent gates, all evaluated *before* any payload is read
//! from the source application (ADR 0003):
//!
//! 1. Secret flags in the offered MIME list (password managers).
//! 2. Per-application exclusion list (case-insensitive substring match).
//! 3. Private mode (user-toggled recording pause).

use crate::config::{compile_ignore_pattern, PrivacyConfig};
use crate::model::{ClipboardData, ContentKind};

/// MIME types that mark clipboard content as secret. If any of these
/// appears in the TARGETS list, the content must never be read or stored.
pub const SECRET_FLAG_MIMES: &[&str] = &[
    "x-kde-passwordmanagerhint",
    "application/x-kde-passwordmanagerhint",
    "org.nspasteboard.concealedtype",
    "application/x-nspasteboard-concealed-type",
    "application/x-qt-windows-mime;value=\"clipboard viewer ignore\"",
];

/// Conservative marker fragments used by password managers and clipboard
/// bridges. Matching is case-insensitive and ignores MIME parameters where
/// safe, because several open-source clients emit slightly different forms.
const SECRET_MIME_MARKERS: &[&str] = &[
    "passwordmanagerhint",
    "concealedtype",
    "clipboard viewer ignore",
];

/// Upper bounds for untrusted clipboard metadata supplied by desktop bridges.
pub const MAX_OFFERED_MIMES: usize = 128;
/// Maximum UTF-8 byte length of one advertised MIME string.
pub const MAX_MIME_LEN: usize = 256;
/// Maximum Unicode scalar count of a reported source application name.
pub const MAX_SOURCE_APP_LEN: usize = 256;

/// Applications excluded by default. Matched case-insensitively as a
/// substring of the reported source application name.
pub const DEFAULT_EXCLUDED_APPS: &[&str] = &[
    "keepassxc",
    "bitwarden",
    "1password",
    "org.keepassxc",
    "com.bitwarden",
    "secrets", // gnome-keyring prompts etc.
];

/// Decision returned by the privacy engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Content may be read and stored.
    Allow,
    /// Content carries a secret flag; never read payloads.
    RejectSecretFlag,
    /// Source application is on the exclusion list.
    RejectExcludedApp,
    /// The focused window's title contains an excluded phrase.
    RejectWindowTitle,
    /// Private mode is active; recording paused.
    RejectPrivateMode,
    /// Backend metadata is malformed or exceeds a safe bound.
    RejectMalformedMetadata,
    /// The content itself fails a user filter (kind, length, whitespace or
    /// an ignore pattern). Only ever returned by `evaluate_content`.
    RejectFilter,
    /// The text looks like a secret and `sensitive_policy` is `drop`. Only
    /// ever returned by `evaluate_content`.
    RejectSensitive,
}

impl Verdict {
    /// True when the content may be stored.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Verdict::Allow)
    }
}

/// User-defined rules judged on the content itself, after the MIME gate
/// allowed it to be read. Built from `PrivacyConfig` with the patterns
/// compiled once.
#[derive(Debug, Clone, Default)]
pub struct ContentFilters {
    /// Minimum trimmed length in characters for text-like entries.
    pub min_text_length: usize,
    /// Drop text that is only whitespace.
    pub ignore_whitespace_only: bool,
    /// Compiled ignore patterns.
    pub patterns: Vec<regex::Regex>,
    /// Kinds to record; `None` means every kind.
    pub kinds: Option<Vec<ContentKind>>,
    /// `sensitive_policy = "drop"`: text `crate::sensitive` flags is refused.
    pub drop_sensitive: bool,
}

impl ContentFilters {
    /// Compile the filters of a configuration. Fails only on a pattern the
    /// configuration validation would have refused as well.
    pub fn from_config(config: &PrivacyConfig) -> crate::error::Result<Self> {
        let patterns = config
            .ignore_patterns
            .iter()
            .map(|p| compile_ignore_pattern(p))
            .collect::<crate::error::Result<Vec<_>>>()?;
        let kinds = if config.capture_kinds.is_empty() {
            None
        } else {
            Some(
                config
                    .capture_kinds
                    .iter()
                    .map(|k| ContentKind::parse(k))
                    .collect(),
            )
        };
        Ok(Self {
            drop_sensitive: config.sensitive_policy == "drop",
            min_text_length: config.min_text_length,
            ignore_whitespace_only: config.ignore_whitespace_only,
            patterns,
            kinds,
        })
    }
}

/// Stateless privacy policy evaluator. Construct once at daemon startup
/// from configuration; cheap to query on every clipboard event.
#[derive(Debug, Clone)]
pub struct PrivacyEngine {
    excluded_apps: Vec<String>,
    private_mode: std::sync::Arc<std::sync::atomic::AtomicBool>,
    filters: ContentFilters,
    /// Lowercased phrases; a focused window whose title contains one
    /// pauses recording for that copy.
    excluded_titles: Vec<String>,
}

impl PrivacyEngine {
    /// Build an engine with the given exclusion list. Default exclusions
    /// are always active; user entries extend them.
    pub fn new(user_excluded: &[String]) -> Self {
        let mut excluded: Vec<String> = DEFAULT_EXCLUDED_APPS
            .iter()
            .map(|s| s.to_ascii_lowercase())
            .collect();
        for app in user_excluded {
            let a = app.trim().to_ascii_lowercase();
            if !a.is_empty() && !excluded.contains(&a) {
                excluded.push(a);
            }
        }
        Self {
            excluded_apps: excluded,
            private_mode: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            filters: ContentFilters {
                min_text_length: 1,
                ignore_whitespace_only: true,
                ..ContentFilters::default()
            },
            excluded_titles: Vec::new(),
        }
    }

    /// Attach content filters (kinds, length, whitespace, ignore patterns).
    pub fn with_filters(mut self, filters: ContentFilters) -> Self {
        self.filters = filters;
        self
    }

    /// Phrases that, found in the focused window's title, keep a copy out
    /// of the history (case-insensitive substrings, e.g. "Online Banking").
    pub fn with_excluded_titles(mut self, titles: &[String]) -> Self {
        self.excluded_titles = titles
            .iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        self
    }

    /// The window-title gate, judged before any payload is read. An
    /// unknown title (plain Wayland, no focus) never blocks a copy.
    pub fn evaluate_title(&self, title: Option<&str>) -> Verdict {
        let Some(title) = title.filter(|_| !self.excluded_titles.is_empty()) else {
            return Verdict::Allow;
        };
        let title = title.to_lowercase();
        if self
            .excluded_titles
            .iter()
            .any(|phrase| title.contains(phrase.as_str()))
        {
            Verdict::RejectWindowTitle
        } else {
            Verdict::Allow
        }
    }

    /// Second gate, judged on the payloads once they have been read: the
    /// entry's kind, the trimmed text length, whitespace-only text and the
    /// user's ignore patterns. Metadata-only rules live in `evaluate`.
    pub fn evaluate_content(&self, data: &ClipboardData) -> Verdict {
        let kind = data.classify();
        if let Some(kinds) = &self.filters.kinds {
            if !kinds.contains(&kind) {
                return Verdict::RejectFilter;
            }
        }
        let Some(text) = data.text() else {
            return Verdict::Allow;
        };
        let trimmed = text.trim();
        if self.filters.ignore_whitespace_only && trimmed.is_empty() {
            return Verdict::RejectFilter;
        }
        if trimmed.chars().count() < self.filters.min_text_length {
            return Verdict::RejectFilter;
        }
        if self.filters.patterns.iter().any(|p| p.is_match(trimmed)) {
            return Verdict::RejectFilter;
        }
        if self.filters.drop_sensitive && crate::sensitive::detect(trimmed).is_some() {
            return Verdict::RejectSensitive;
        }
        Verdict::Allow
    }

    /// Toggle private mode (recording pause).
    pub fn set_private_mode(&self, on: bool) {
        self.private_mode
            .store(on, std::sync::atomic::Ordering::SeqCst);
    }

    /// Whether private mode is currently active.
    pub fn private_mode(&self) -> bool {
        self.private_mode.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Current exclusion list (lowercased), for settings UI.
    pub fn excluded_apps(&self) -> &[String] {
        &self.excluded_apps
    }

    /// True if the offered MIME list contains a secret flag.
    ///
    /// This MUST be checked before reading any payload (race-condition
    /// guard, ADR 0003).
    pub fn has_secret_flag(offered_mimes: &[String]) -> bool {
        offered_mimes.iter().any(|mime| {
            let normalized = mime.trim().to_ascii_lowercase();
            SECRET_FLAG_MIMES
                .iter()
                .any(|flag| normalized == *flag || normalized.starts_with(&format!("{flag};")))
                || SECRET_MIME_MARKERS
                    .iter()
                    .any(|marker| normalized.contains(marker))
        })
    }

    /// Evaluate captured data against the policy. The caller must pass
    /// the data with `offered_mimes` populated; payloads may still be
    /// empty at this point (and SHOULD be, until Allow is returned).
    pub fn evaluate(&self, data: &ClipboardData) -> Verdict {
        if self.private_mode() {
            return Verdict::RejectPrivateMode;
        }
        if data.offered_mimes.len() > MAX_OFFERED_MIMES
            || data
                .offered_mimes
                .iter()
                .any(|mime| mime.len() > MAX_MIME_LEN)
            || data
                .source_app
                .as_deref()
                .is_some_and(|app| app.chars().count() > MAX_SOURCE_APP_LEN)
        {
            return Verdict::RejectMalformedMetadata;
        }
        if Self::has_secret_flag(&data.offered_mimes) {
            return Verdict::RejectSecretFlag;
        }
        if let Some(app) = &data.source_app {
            let app_lower = app.to_ascii_lowercase();
            if self
                .excluded_apps
                .iter()
                .any(|ex| app_lower.contains(ex.as_str()))
            {
                return Verdict::RejectExcludedApp;
            }
        }
        Verdict::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MimePayload, Selection};

    fn data(mimes: &[&str], app: Option<&str>) -> ClipboardData {
        ClipboardData {
            selection: Selection::Clipboard,
            payloads: vec![MimePayload {
                mime: "text/plain".into(),
                data: b"secret-password-123".to_vec(),
            }],
            offered_mimes: mimes.iter().map(|s| s.to_string()).collect(),
            source_app: app.map(|s| s.to_string()),
        }
    }

    #[test]
    fn allows_ordinary_text() {
        let eng = PrivacyEngine::new(&[]);
        let d = data(&["text/plain", "UTF8_STRING"], Some("firefox"));
        assert_eq!(eng.evaluate(&d), Verdict::Allow);
    }

    #[test]
    fn rejects_kde_password_hint() {
        let eng = PrivacyEngine::new(&[]);
        let d = data(
            &["x-kde-passwordManagerHint", "text/plain"],
            Some("keepassxc"),
        );
        assert_eq!(eng.evaluate(&d), Verdict::RejectSecretFlag);
    }

    #[test]
    fn rejects_concealed_type_variants() {
        let eng = PrivacyEngine::new(&[]);
        for flag in [
            "org.nspasteboard.ConcealedType",
            "application/x-nspasteboard-concealed-type",
        ] {
            let d = data(&[flag, "text/plain"], None);
            assert_eq!(eng.evaluate(&d), Verdict::RejectSecretFlag, "flag: {flag}");
        }
    }

    #[test]
    fn secret_flag_check_is_case_insensitive_and_parameter_tolerant() {
        assert!(PrivacyEngine::has_secret_flag(&[
            "X-KDE-PASSWORDMANAGERHINT".to_string(),
            "application/x-kde-passwordManagerHint;value=ignore".to_string(),
            "org.nspasteboard.ConcealedType".to_string(),
            "application/x-qt-windows-mime;value=\"Clipboard Viewer Ignore\"".to_string(),
        ]));
    }

    #[test]
    fn ordinary_mime_parameters_are_not_secret() {
        assert!(!PrivacyEngine::has_secret_flag(&[
            "text/plain;charset=utf-8".to_string(),
            "text/html".to_string(),
        ]));
    }

    #[test]
    fn rejects_default_excluded_apps() {
        let eng = PrivacyEngine::new(&[]);
        for app in [
            "KeePassXC",
            "org.keepassxc.KeePassXC",
            "Bitwarden",
            "1Password",
        ] {
            let d = data(&["text/plain"], Some(app));
            assert_eq!(eng.evaluate(&d), Verdict::RejectExcludedApp, "app: {app}");
        }
    }

    #[test]
    fn rejects_user_excluded_app() {
        let eng = PrivacyEngine::new(&["MyBankApp".to_string()]);
        let d = data(&["text/plain"], Some("mybankapp.desktop"));
        assert_eq!(eng.evaluate(&d), Verdict::RejectExcludedApp);
    }

    #[test]
    fn malformed_metadata_is_rejected() {
        let eng = PrivacyEngine::new(&[]);
        let too_many = (0..=MAX_OFFERED_MIMES)
            .map(|_| "text/plain".to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            eng.evaluate(&data(
                &too_many.iter().map(String::as_str).collect::<Vec<_>>(),
                None
            )),
            Verdict::RejectMalformedMetadata
        );
        let long_mime = ["x".repeat(MAX_MIME_LEN + 1)];
        assert_eq!(
            eng.evaluate(&data(
                &long_mime.iter().map(String::as_str).collect::<Vec<_>>(),
                None
            )),
            Verdict::RejectMalformedMetadata
        );
        let long_app = "x".repeat(MAX_SOURCE_APP_LEN + 1);
        assert_eq!(
            eng.evaluate(&data(&["text/plain"], Some(&long_app))),
            Verdict::RejectMalformedMetadata
        );
    }

    #[test]
    fn private_mode_rejects_everything() {
        let eng = PrivacyEngine::new(&[]);
        eng.set_private_mode(true);
        let d = data(&["text/plain"], Some("firefox"));
        assert_eq!(eng.evaluate(&d), Verdict::RejectPrivateMode);
        eng.set_private_mode(false);
        assert_eq!(eng.evaluate(&d), Verdict::Allow);
    }

    #[test]
    fn private_mode_takes_priority_over_secret_flag() {
        // Any rejection is fine; private mode is simply evaluated first.
        let eng = PrivacyEngine::new(&[]);
        eng.set_private_mode(true);
        let d = data(&["x-kde-passwordManagerHint"], Some("keepassxc"));
        assert!(!eng.evaluate(&d).is_allowed());
    }

    fn text_data(text: &str) -> ClipboardData {
        ClipboardData {
            selection: Selection::Clipboard,
            payloads: vec![MimePayload::new("text/plain", text.as_bytes())],
            offered_mimes: vec!["text/plain".into()],
            source_app: None,
        }
    }

    #[test]
    fn default_filters_only_drop_empty_text() {
        let eng = PrivacyEngine::new(&[]);
        assert_eq!(eng.evaluate_content(&text_data("x")), Verdict::Allow);
        assert_eq!(
            eng.evaluate_content(&text_data("   \n\t")),
            Verdict::RejectFilter
        );
        let image = ClipboardData {
            selection: Selection::Clipboard,
            payloads: vec![MimePayload::new("image/png", vec![1, 2, 3])],
            offered_mimes: vec!["image/png".into()],
            source_app: None,
        };
        assert_eq!(eng.evaluate_content(&image), Verdict::Allow);
    }

    #[test]
    fn user_filters_judge_kind_length_and_patterns() {
        let config = PrivacyConfig {
            min_text_length: 3,
            ignore_whitespace_only: true,
            ignore_patterns: vec!["^\\d{16}$".into(), "(?i)^secret:".into()],
            capture_kinds: vec!["text".into(), "image".into()],
            ..PrivacyConfig::default()
        };
        let eng =
            PrivacyEngine::new(&[]).with_filters(ContentFilters::from_config(&config).unwrap());
        assert_eq!(eng.evaluate_content(&text_data("hello")), Verdict::Allow);
        assert_eq!(
            eng.evaluate_content(&text_data("ab")),
            Verdict::RejectFilter
        );
        assert_eq!(
            eng.evaluate_content(&text_data("1234567890123456")),
            Verdict::RejectFilter
        );
        assert_eq!(
            eng.evaluate_content(&text_data("Secret: token")),
            Verdict::RejectFilter
        );
        assert_eq!(
            eng.evaluate_content(&text_data("  1234567890123456  ")),
            Verdict::RejectFilter,
            "patterns see the trimmed text"
        );
        // A link is not in capture_kinds.
        assert_eq!(
            eng.evaluate_content(&text_data("https://example.org/page")),
            Verdict::RejectFilter
        );
        let image = ClipboardData {
            selection: Selection::Clipboard,
            payloads: vec![MimePayload::new("image/png", vec![1, 2, 3])],
            offered_mimes: vec!["image/png".into()],
            source_app: None,
        };
        assert_eq!(eng.evaluate_content(&image), Verdict::Allow);
        // Turning the whitespace rule off records blanks (min length 0).
        let lax = PrivacyConfig {
            min_text_length: 0,
            ignore_whitespace_only: false,
            ..PrivacyConfig::default()
        };
        let eng = PrivacyEngine::new(&[]).with_filters(ContentFilters::from_config(&lax).unwrap());
        assert_eq!(eng.evaluate_content(&text_data("   ")), Verdict::Allow);
    }

    #[test]
    fn drop_policy_refuses_secrets_at_the_filter_gate() {
        let eng = PrivacyEngine::new(&[]);
        assert_eq!(
            eng.evaluate_content(&text_data("AKIAIOSFODNN7EXAMPLE")),
            Verdict::Allow,
            "mask (the default) records the entry and masks it later"
        );
        let config = PrivacyConfig {
            sensitive_policy: "drop".into(),
            ..PrivacyConfig::default()
        };
        let eng =
            PrivacyEngine::new(&[]).with_filters(ContentFilters::from_config(&config).unwrap());
        assert_eq!(
            eng.evaluate_content(&text_data("AKIAIOSFODNN7EXAMPLE")),
            Verdict::RejectSensitive
        );
        assert_eq!(
            eng.evaluate_content(&text_data("just a note")),
            Verdict::Allow
        );
    }

    #[test]
    fn window_titles_are_matched_as_case_insensitive_substrings() {
        let eng = PrivacyEngine::new(&[]).with_excluded_titles(&[
            "Online Banking".into(),
            "   ".into(),
            "PayPal".into(),
        ]);
        assert_eq!(
            eng.evaluate_title(Some("Online banking — Mozilla Firefox")),
            Verdict::RejectWindowTitle
        );
        assert_eq!(
            eng.evaluate_title(Some("paypal.com")),
            Verdict::RejectWindowTitle
        );
        assert_eq!(eng.evaluate_title(Some("Weather")), Verdict::Allow);
        assert_eq!(eng.evaluate_title(None), Verdict::Allow);
        let none = PrivacyEngine::new(&[]);
        assert_eq!(none.evaluate_title(Some("Online Banking")), Verdict::Allow);
    }
}
