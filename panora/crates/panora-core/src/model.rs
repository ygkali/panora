// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Data model: clipboard entries, MIME payloads, content classification.

use serde::{Deserialize, Serialize};

/// Which selection an entry came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Selection {
    /// The Ctrl+C / Ctrl+V clipboard.
    Clipboard,
    /// The X11/Wayland primary selection (mouse highlight, middle-click).
    Primary,
}

impl Selection {
    /// Stable string form used in the database and IPC.
    pub fn as_str(&self) -> &'static str {
        match self {
            Selection::Clipboard => "clipboard",
            Selection::Primary => "primary",
        }
    }
}

/// High-level content classification, derived from the offered MIME types.
/// Drives UI badges, icons and preview rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContentKind {
    /// Plain text, source code, or any unclassified text.
    Text,
    /// Rich text (HTML/RTF present alongside plain text).
    RichText,
    /// One or more URLs.
    Link,
    /// Raster or vector image data.
    Image,
    /// A list of file URIs (copied files in a file manager).
    FileList,
    /// Text that parses as a color (#rrggbb, rgb(), named colors).
    Color,
    /// An opaque or otherwise unclassified binary payload.
    Binary,
}

impl ContentKind {
    /// Stable string form used in the database.
    pub fn as_str(&self) -> &'static str {
        match self {
            ContentKind::Text => "text",
            ContentKind::RichText => "richtext",
            ContentKind::Link => "link",
            ContentKind::Image => "image",
            ContentKind::FileList => "files",
            ContentKind::Color => "color",
            ContentKind::Binary => "binary",
        }
    }

    /// Parse back from the database string form (infallible; unknown
    /// values degrade to `Text`).
    pub fn parse(s: &str) -> Self {
        match s {
            "richtext" => ContentKind::RichText,
            "link" => ContentKind::Link,
            "image" => ContentKind::Image,
            "files" => ContentKind::FileList,
            "color" => ContentKind::Color,
            "binary" => ContentKind::Binary,
            _ => ContentKind::Text,
        }
    }
}

/// A single MIME payload belonging to an entry. One clipboard event can
/// offer several formats at once (e.g. text/plain + text/html).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MimePayload {
    /// MIME type, e.g. "text/plain" or "image/png".
    pub mime: String,
    /// Raw bytes as offered by the source application.
    pub data: Vec<u8>,
}

/// A captured clipboard event before it is persisted. Produced by
/// backends, consumed by the privacy engine and the store.
#[derive(Debug, Clone)]
pub struct ClipboardData {
    /// Selection this data came from.
    pub selection: Selection,
    /// All MIME payloads offered by the source, in preference order.
    pub payloads: Vec<MimePayload>,
    /// All MIME types advertised by the source (the TARGETS list).
    /// Read *before* any payload so privacy flags can be checked first.
    pub offered_mimes: Vec<String>,
    /// Best-effort name of the source application (may be None).
    pub source_app: Option<String>,
}

impl ClipboardData {
    /// Total size of all payloads in bytes.
    pub fn total_size(&self) -> usize {
        self.payloads.iter().map(|p| p.data.len()).sum()
    }

    /// First payload matching the given MIME prefix (e.g. "text/").
    pub fn payload_for(&self, mime_prefix: &str) -> Option<&MimePayload> {
        self.payloads
            .iter()
            .find(|p| p.mime.starts_with(mime_prefix))
    }

    /// Classify the content from its offered MIME types and text content.
    pub fn classify(&self) -> ContentKind {
        let has = |m: &str| self.offered_mimes.iter().any(|x| x == m);
        let has_prefix = |p: &str| self.offered_mimes.iter().any(|x| x.starts_with(p));

        if has_prefix("image/") {
            return ContentKind::Image;
        }
        if has("x-special/gnome-copied-files") || has("application/vnd.kde.cutsel") {
            return ContentKind::FileList;
        }
        if has("text/uri-list") {
            // uri-list alone means links; with gnome-copied-files it is files.
            return ContentKind::Link;
        }
        if has("text/html") || has("text/rtf") || has("application/rtf") {
            return ContentKind::RichText;
        }
        if has("text/x-color") || has("application/x-color") {
            return ContentKind::Color;
        }
        if let Some(text) = self.text() {
            let t = text.trim();
            if looks_like_color(t) {
                return ContentKind::Color;
            }
            if looks_like_url(t) {
                return ContentKind::Link;
            }
        }
        if self.payloads.iter().any(|p| !is_text_mime(&p.mime)) {
            return ContentKind::Binary;
        }
        ContentKind::Text
    }

    /// Extract the best plain-text representation, if any.
    pub fn text(&self) -> Option<String> {
        for mime in [
            "text/plain;charset=utf-8",
            "text/plain",
            "UTF8_STRING",
            "STRING",
            "TEXT",
        ] {
            if let Some(p) = self.payloads.iter().find(|p| p.mime == mime) {
                if let Ok(s) = std::str::from_utf8(&p.data) {
                    return Some(s.trim_end_matches('\0').to_string());
                }
            }
        }
        None
    }
}

/// A persisted history entry as returned by the store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Database row id.
    pub id: i64,
    /// BLAKE3 content hash (hex), used for deduplication.
    pub content_hash: String,
    /// Short preview text (first ~500 chars) for list rendering and FTS.
    pub preview: String,
    /// High-level classification.
    pub kind: ContentKind,
    /// Primary MIME type (first payload).
    pub primary_mime: String,
    /// Total payload size in bytes.
    pub size_bytes: i64,
    /// Best-effort source application name.
    pub source_app: Option<String>,
    /// Unix timestamp (seconds) of first capture.
    pub created_at: i64,
    /// Unix timestamp of most recent capture (re-copy bumps this).
    pub last_seen_at: i64,
    /// Whether the entry is pinned (survives cleanup, sorts first).
    pub pinned: bool,
    /// Which selection it came from.
    pub selection: Selection,
    // --- sync-ready fields (ADR 0002); unused by v1.0 logic ---
    /// Originating device id (random per install; hex).
    pub device_id: String,
    /// Lamport clock value for total ordering across devices.
    pub lamport: i64,
    /// Tombstone flag: entry deleted locally, kept for sync propagation.
    pub deleted: bool,
}

/// True for canonical text MIME names and X11 legacy text targets.
fn is_text_mime(mime: &str) -> bool {
    mime.starts_with("text/") || matches!(mime, "UTF8_STRING" | "STRING" | "TEXT")
}

/// True if the text looks like a single color value.
fn looks_like_color(t: &str) -> bool {
    let t = t.trim();
    if t.len() > 32 {
        return false;
    }
    // #rgb, #rrggbb, #rrggbbaa
    if let Some(hex) = t.strip_prefix('#') {
        return matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    // rgb(r, g, b) / rgba(r, g, b, a)
    let lower = t.to_ascii_lowercase();
    if (lower.starts_with("rgb(") || lower.starts_with("rgba(")) && lower.ends_with(')') {
        return true;
    }
    false
}

/// True if the text looks like a single URL.
fn looks_like_url(t: &str) -> bool {
    let t = t.trim();
    if t.len() > 2048 || t.contains(char::is_whitespace) {
        return false;
    }
    t.starts_with("http://")
        || t.starts_with("https://")
        || t.starts_with("ftp://")
        || t.starts_with("file://")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data_with(mimes: &[&str], text: Option<&str>) -> ClipboardData {
        let mut payloads = Vec::new();
        if let Some(t) = text {
            payloads.push(MimePayload {
                mime: "text/plain".into(),
                data: t.as_bytes().to_vec(),
            });
        }
        ClipboardData {
            selection: Selection::Clipboard,
            payloads,
            offered_mimes: mimes.iter().map(|s| s.to_string()).collect(),
            source_app: None,
        }
    }

    #[test]
    fn classify_legacy_x11_text() {
        let d = data_with(&["UTF8_STRING"], Some("hello"));
        assert_eq!(d.classify(), ContentKind::Text);
    }

    #[test]
    fn classify_plain_text() {
        let d = data_with(&["text/plain"], Some("hello world"));
        assert_eq!(d.classify(), ContentKind::Text);
    }

    #[test]
    fn classify_image() {
        let d = data_with(&["image/png", "text/plain"], None);
        assert_eq!(d.classify(), ContentKind::Image);
    }

    #[test]
    fn classify_files() {
        let d = data_with(&["x-special/gnome-copied-files", "text/uri-list"], None);
        assert_eq!(d.classify(), ContentKind::FileList);
    }

    #[test]
    fn classify_link_from_uri_list() {
        let d = data_with(&["text/uri-list"], None);
        assert_eq!(d.classify(), ContentKind::Link);
    }

    #[test]
    fn classify_link_from_text() {
        let d = data_with(&["text/plain"], Some("https://example.com/page"));
        assert_eq!(d.classify(), ContentKind::Link);
    }

    #[test]
    fn classify_richtext() {
        let d = data_with(&["text/html", "text/plain"], Some("<b>hi</b>"));
        assert_eq!(d.classify(), ContentKind::RichText);
    }

    #[test]
    fn classify_color_hex() {
        for c in ["#fff", "#a1b2c3", "#a1b2c3d4"] {
            let d = data_with(&["text/plain"], Some(c));
            assert_eq!(d.classify(), ContentKind::Color, "failed for {c}");
        }
    }

    #[test]
    fn classify_color_rgb_fn() {
        let d = data_with(&["text/plain"], Some("rgb(12, 34, 56)"));
        assert_eq!(d.classify(), ContentKind::Color);
    }

    #[test]
    fn not_color_when_long() {
        let d = data_with(&["text/plain"], Some("#this-is-not-a-color-at-all-really"));
        assert_eq!(d.classify(), ContentKind::Text);
    }

    #[test]
    fn text_extraction_prefers_utf8() {
        let d = ClipboardData {
            selection: Selection::Clipboard,
            payloads: vec![
                MimePayload {
                    mime: "text/html".into(),
                    data: b"<b>x</b>".to_vec(),
                },
                MimePayload {
                    mime: "text/plain".into(),
                    data: "merhaba".as_bytes().to_vec(),
                },
            ],
            offered_mimes: vec!["text/html".into(), "text/plain".into()],
            source_app: None,
        };
        assert_eq!(d.text().as_deref(), Some("merhaba"));
    }

    #[test]
    fn selection_roundtrip() {
        assert_eq!(Selection::Clipboard.as_str(), "clipboard");
        assert_eq!(Selection::Primary.as_str(), "primary");
    }
}
