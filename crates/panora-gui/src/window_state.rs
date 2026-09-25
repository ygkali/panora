// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Remembers the popup's last size across sessions (UI-24): a small text
//! file next to `config.toml`, the same pattern `welcome::marker_path`
//! uses for "first run seen". Window geometry is remembered *state*, not a
//! user preference, so it does not belong in `config.toml` (and never
//! shows up in `panora-cli config get`).

use panora_core::config::config_path;
use std::path::PathBuf;

/// The design default, also `build()`'s fallback when nothing was saved
/// yet or the saved file could not be read.
const DEFAULT_SIZE: (i32, i32) = (420, 660);
/// Matches the window's own `width_request`/`height_request`: a saved size
/// smaller than this could not have come from a real resize.
const MIN_SIZE: (i32, i32) = (340, 420);
/// A generous sanity cap so a corrupted or hand-edited file cannot make the
/// popup open comically (or unusably) large.
const MAX_SIZE: (i32, i32) = (4000, 4000);

fn state_path() -> PathBuf {
    config_path().with_file_name("window-size")
}

/// The size to open the popup at.
pub fn load() -> (i32, i32) {
    let text = std::fs::read_to_string(state_path()).unwrap_or_default();
    parse_size(&text).unwrap_or(DEFAULT_SIZE)
}

/// Best-effort: nothing about remembering the window size is worth
/// surfacing a failure to the user over. The next open just falls back to
/// whatever was last saved successfully, or the default.
pub fn save(width: i32, height: i32) {
    let _ = std::fs::write(state_path(), format!("{width}x{height}"));
}

/// `"420x660"` -> `Some((420, 660))`. `None` for anything unparsable or
/// outside the sane bounds a real window could have.
fn parse_size(text: &str) -> Option<(i32, i32)> {
    let (w, h) = text.trim().split_once('x')?;
    let w: i32 = w.parse().ok()?;
    let h: i32 = h.parse().ok()?;
    let in_range = |v: i32, (min, max): (i32, i32)| (min..=max).contains(&v);
    if in_range(w, (MIN_SIZE.0, MAX_SIZE.0)) && in_range(h, (MIN_SIZE.1, MAX_SIZE.1)) {
        Some((w, h))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_saved_format() {
        assert_eq!(parse_size("420x660"), Some((420, 660)));
        assert_eq!(parse_size(" 900x1200 \n"), Some((900, 1200)));
    }

    #[test]
    fn rejects_garbage_and_falls_back() {
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("420"), None);
        assert_eq!(parse_size("wide x tall"), None);
        assert_eq!(parse_size("420x-5"), None);
    }

    #[test]
    fn rejects_sizes_outside_the_sane_range() {
        // Smaller than the window's own width_request/height_request.
        assert_eq!(parse_size("100x100"), None);
        // A corrupted or malicious value should not be trusted verbatim.
        assert_eq!(parse_size("999999x999999"), None);
    }
}
