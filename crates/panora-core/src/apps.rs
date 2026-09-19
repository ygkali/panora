// Copyright (C) 2026 Panora contributors
// SPDX-License-Identifier: GPL-3.0-only

//! Knowledge about application classes that need special treatment.
//!
//! Terminal emulators treat Ctrl+V as a control character and paste with
//! Ctrl+Shift+V instead, so instant paste must pick the combination by the
//! focused application. The names here are matched against whatever the
//! backend reports: an X11 `WM_CLASS` class (`Alacritty`, `gnome-terminal`),
//! a Wayland/GNOME app id (`org.gnome.Terminal`) or a desktop file id
//! (`org.gnome.Ptyxis.desktop`).

/// Terminal emulators known by their exact (case-insensitive) class, app id
/// or the last segment of a reverse-DNS id. Anything whose name contains
/// `terminal` or ends in `term` counts too, so distribution forks are covered.
pub const TERMINAL_APPS: &[&str] = &[
    "alacritty",
    "blackbox",
    "com.mitchellh.ghostty",
    "com.raggesilver.blackbox",
    "console",
    "contour",
    "dev.warp.warp",
    "foot",
    "footclient",
    "ghostty",
    "guake",
    "hyper",
    "kgx",
    "kitty",
    "konsole",
    "org.codeberg.dnkl.foot",
    "org.gnome.console",
    "org.gnome.ptyxis",
    "org.gnome.terminal",
    "org.kde.konsole",
    "org.kde.yakuake",
    "org.wezfurlong.wezterm",
    "ptyxis",
    "rio",
    "rxvt",
    "sakura",
    "st",
    "st-256color",
    "tabby",
    "tilda",
    "tilix",
    "urxvt",
    "uxterm",
    "warp",
    "wave",
    "wezterm",
    "xterm",
    "yakuake",
];

/// True when `app` names a terminal emulator, so a paste should be sent as
/// Ctrl+Shift+V rather than Ctrl+V.
pub fn is_terminal(app: &str) -> bool {
    let name = app.trim().to_ascii_lowercase();
    let name = name.strip_suffix(".desktop").unwrap_or(&name);
    if name.is_empty() {
        return false;
    }
    let last = name.rsplit('.').next().unwrap_or(name);
    for candidate in [name, last] {
        if TERMINAL_APPS.contains(&candidate)
            || candidate.contains("terminal")
            || candidate.ends_with("term")
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_common_terminals_in_every_naming_style() {
        for app in [
            "org.gnome.Terminal",
            "org.gnome.Terminal.desktop",
            "gnome-terminal-server",
            "Gnome-terminal",
            "org.gnome.Ptyxis",
            "kgx",
            "org.gnome.Console",
            "konsole",
            "org.kde.konsole.desktop",
            "Alacritty",
            "kitty",
            "foot",
            "org.codeberg.dnkl.foot",
            "org.wezfurlong.wezterm",
            "xfce4-terminal",
            "mate-terminal",
            "io.elementary.terminal",
            "com.gexperts.Tilix",
            "XTerm",
            "st-256color",
            "cool-retro-term",
            "com.mitchellh.ghostty",
        ] {
            assert!(is_terminal(app), "{app} should count as a terminal");
        }
    }

    #[test]
    fn leaves_ordinary_applications_alone() {
        for app in [
            "firefox",
            "org.mozilla.firefox",
            "steam",
            "Steam",
            "org.gnome.Nautilus",
            "gnome-text-editor",
            "code",
            "Code",
            "libreoffice-writer",
            "org.telegram.desktop",
            "",
            "   ",
            "stardew-valley",
            "system-monitor",
        ] {
            assert!(!is_terminal(app), "{app} must not count as a terminal");
        }
    }
}
