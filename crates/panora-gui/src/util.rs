// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Formatting helpers and the IPC bridge used by every dialog.

use gtk4 as gtk;
use libadwaita as adw;
use libadwaita::prelude::*;
use panora_core::error::Result;
use panora_core::i18n::{fill, Strings};
use panora_core::ipc::{Request, ResponseData};
use panora_core::model::ContentKind;
use std::time::{SystemTime, UNIX_EPOCH};

/// One IPC round trip, blocking. The daemon is local and answers small
/// requests in well under a millisecond, so pin, delete, status and the
/// like call this from the GTK thread; anything that carries a page or a
/// payload goes through `call_async` instead.
#[cfg(not(feature = "fixture"))]
pub fn call(request: &Request) -> Result<ResponseData> {
    panora_core::ipc::client::call(request)
}

/// Canned daemon for UI work without panod (see `fixture.rs`).
#[cfg(feature = "fixture")]
pub fn call(request: &Request) -> Result<ResponseData> {
    crate::fixture::call(request)
}

/// Run `work` on a worker thread and hand its result to `done` on the GTK
/// main loop. Everything that takes time (a daemon round trip carrying a
/// page or a payload, an image decode) goes through here so the popup
/// never waits on it.
pub fn spawn<T, W, D>(work: W, done: D)
where
    T: Send + 'static,
    W: FnOnce() -> T + Send + 'static,
    D: FnOnce(T) + 'static,
{
    glib::spawn_future_local(async move {
        if let Ok(value) = gtk::gio::spawn_blocking(work).await {
            done(value);
        }
    });
}

/// `call` on a worker thread; `done` runs on the GTK main loop.
pub fn call_async<D>(request: Request, done: D)
where
    D: FnOnce(Result<ResponseData>) + 'static,
{
    spawn(move || call(&request), done);
}

/// The quiet zone around a QR code, in modules.
const QR_QUIET: usize = 4;

/// `text` as a black-on-white QR code, `scale` pixels per module; `None`
/// when it is too long for one.
pub fn qr_picture(text: &str, scale: usize) -> Option<gtk::Picture> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let modules = code.width();
    let side = (modules + 2 * QR_QUIET) * scale;
    let mut pixels = vec![255u8; side * side * 3];
    for (index, color) in code.to_colors().iter().enumerate() {
        if *color != qrcode::Color::Dark {
            continue;
        }
        let (mx, my) = (index % modules + QR_QUIET, index / modules + QR_QUIET);
        for y in my * scale..(my + 1) * scale {
            for x in mx * scale..(mx + 1) * scale {
                let at = (y * side + x) * 3;
                pixels[at..at + 3].copy_from_slice(&[0, 0, 0]);
            }
        }
    }
    let side_px = i32::try_from(side).ok()?;
    let texture = gtk::gdk::MemoryTexture::new(
        side_px,
        side_px,
        gtk::gdk::MemoryFormat::R8g8b8,
        &glib::Bytes::from_owned(pixels),
        side * 3,
    );
    let picture = gtk::Picture::for_paintable(&texture);
    picture.set_size_request(side_px, side_px);
    picture.set_can_shrink(false);
    picture.set_halign(gtk::Align::Center);
    Some(picture)
}

/// Current Unix time in seconds.
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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

/// Human readable byte count (I18N-02: uses `s.decimal_separator`, a comma
/// rather than a period in a language that formats numbers that way).
pub fn format_size(s: &Strings, size: i64) -> String {
    let formatted = if size < 1024 {
        return format!("{size} B");
    } else if size < 1024 * 1024 {
        format!("{:.1} KiB", size as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", size as f64 / 1_048_576.0)
    };
    formatted.replacen('.', s.decimal_separator, 1)
}

/// Relative timestamp for the card header.
pub fn relative_time(s: &Strings, timestamp: i64) -> String {
    relative_time_at(s, timestamp, unix_now())
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
        let en = Language::English.strings();
        assert_eq!(format_size(en, 12), "12 B");
        assert_eq!(format_size(en, 2048), "2.0 KiB");
        assert_eq!(format_size(en, 3 * 1024 * 1024), "3.0 MiB");
    }

    #[test]
    fn sizes_use_the_language_decimal_separator() {
        let tr = Language::Turkish.strings();
        assert_eq!(format_size(tr, 2048), "2,0 KiB");
        assert_eq!(format_size(tr, 12), "12 B");
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
