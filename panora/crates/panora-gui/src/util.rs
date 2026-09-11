// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Formatting helpers and the IPC bridge used by every dialog.

use libadwaita as adw;
use panora_core::error::Result;
use panora_core::i18n::{fill, Strings};
use panora_core::ipc::{Request, ResponseData};
use panora_core::model::ContentKind;
use std::time::{SystemTime, UNIX_EPOCH};

/// One IPC round trip on the GTK thread. The daemon is local, so calls take
/// milliseconds; only image previews are moved to a worker thread.
pub fn call(request: &Request) -> Result<ResponseData> {
    panora_core::ipc::client::call(request)
}

/// Apply the configured colour scheme through libadwaita.
pub fn apply_theme(theme: &str) {
    let scheme = match theme {
        "light" => adw::ColorScheme::ForceLight,
        "dark" => adw::ColorScheme::ForceDark,
        _ => adw::ColorScheme::Default,
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

/// Badge caption for a content kind.
pub fn kind_label(s: &Strings, kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Text => s.kind_text,
        ContentKind::RichText => s.kind_richtext,
        ContentKind::Link => s.kind_link,
        ContentKind::Image => s.kind_image,
        ContentKind::FileList => s.kind_files,
        ContentKind::Color => s.kind_color,
        ContentKind::Binary => s.kind_binary,
    }
}

/// Symbolic icon for a content kind.
pub fn kind_icon(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Text => "text-x-generic-symbolic",
        ContentKind::RichText => "font-x-generic-symbolic",
        ContentKind::Link => "insert-link-symbolic",
        ContentKind::Image => "image-x-generic-symbolic",
        ContentKind::FileList => "folder-symbolic",
        ContentKind::Color => "color-select-symbolic",
        ContentKind::Binary => "application-x-executable-symbolic",
    }
}

/// Human readable byte count.
pub fn format_size(size: i64) -> String {
    if size < 1024 {
        format!("{size} B")
    } else if size < 1024 * 1024 {
        format!("{:.1} KiB", size as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", size as f64 / 1_048_576.0)
    }
}

/// Relative timestamp for the card header.
pub fn relative_time(s: &Strings, timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(timestamp);
    relative_time_at(s, timestamp, now)
}

fn relative_time_at(s: &Strings, timestamp: i64, now: i64) -> String {
    let diff = (now - timestamp).max(0);
    match diff {
        0..=59 => s.time_just_now.to_string(),
        60..=3599 => fill(s.time_minutes, "n", &(diff / 60).to_string()),
        3600..=86_399 => fill(s.time_hours, "n", &(diff / 3600).to_string()),
        86_400..=604_799 => fill(s.time_days, "n", &(diff / 86_400).to_string()),
        _ => fill(s.time_weeks, "n", &(diff / 604_800).to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panora_core::i18n::Language;

    #[test]
    fn sizes() {
        assert_eq!(format_size(12), "12 B");
        assert_eq!(format_size(2048), "2.0 KiB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MiB");
    }

    #[test]
    fn relative_times_in_both_languages() {
        let tr = Language::Turkish.strings();
        let en = Language::English.strings();
        assert_eq!(relative_time_at(tr, 100, 130), "az önce");
        assert_eq!(relative_time_at(en, 0, 120), "2 min ago");
        assert_eq!(relative_time_at(tr, 0, 7200), "2 sa önce");
        assert_eq!(relative_time_at(en, 0, 3 * 604_800), "3 w ago");
        assert_eq!(relative_time_at(en, 500, 100), "just now");
    }
}
