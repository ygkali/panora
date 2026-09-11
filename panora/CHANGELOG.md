# Changelog

## 1.2.0

### Zorin OS 18 / Ubuntu 24.04
- `install.sh` refuses releases older than Ubuntu 24.04 / Debian 13 / Zorin 18 with a clear message, detects a too-old apt `cargo` (1.75) and installs rustup instead, picks the `libglib2.0-0t64` runtime name, installs `libglib2.0-bin` (where `glib-compile-schemas` really lives) and tolerates a broken third-party apt repo.
- GNOME 46 Wayland bridge: `PushMany` carries text + HTML (or uri-list) per change so bridge entries have the same fidelity as native captures; the daemon recognises the echo of its own recall; a failed bridge service is fatal when capture depends on it; backend detection no longer calls the blocking zbus API inside the runtime (this crashed panod at startup on sessions without `XDG_CURRENT_DESKTOP`).
- Extension: loads in Zorin's `zorin` session mode, takes `<Super>v` away from GNOME's notification list while enabled (restored on disable), waits for focus to leave the popup before pasting, uses evdev key codes so Ctrl+V works on any layout, activates the popup with a 25 s timeout and only falls back to spawning when no D-Bus service exists.
- New `panora-doctor` (installed to /usr/bin) diagnoses the session, daemon, extension, D-Bus names, keyring and shortcut conflicts; `scripts/e2e-test.sh` runs a PASS/FAIL functional test of every feature on the real machine; `test-local.sh` runs the doctor first.

### Daemon
- Native X11 backend (x11rb): XFIXES change events instead of 180 ms polling, `ConvertSelection` reads with INCR, panod becomes the selection owner on recall and serves every stored format (text + HTML, images) with INCR for large payloads, XTEST instant paste, re-offer of the last entry when the owning application exits. `xclip` is no longer required.
- Native Wayland backend (wayland-client): `ext-data-control-v1` and `wlr-data-control-v1`, event-driven capture with the MIME list delivered before any payload, multi-format data sources on recall, primary selection support. `wl-clipboard` is no longer required. Consecutive copies of the same type are no longer missed.
- GNOME bridge backend for GNOME ≤ 47 (no data-control): recall and paste through the Shell extension's new `io.panora.GnomeShell1` service; bridge pushes are ignored when a native backend is active so entries are never duplicated.
- Live configuration reload (`ReloadConfig`), preserving private mode; retention runs hourly and on every store; evicted or deleted entries release their encrypted blobs unless another entry still references them; revived entries are indexed for search again.
- `Recall { paste }` synthesizes Ctrl+V after the clipboard is set; `Toggle` activates the GUI over D-Bus; `Status` reports revision, version, protocol and capabilities; SIGTERM shuts down cleanly.
- FTS5 prefix search (`mer` finds `merhaba`), safe against operator injection.

### GUI
- Unique application: Super+V / `panora` / `panora-cli toggle` toggle the popup; D-Bus activation file installed.
- Turkish and English catalogues (`ui.language`), light/dark/system theme (`ui.theme`).
- Settings dialog (history limits, PRIMARY recording, private start, excluded applications, language, theme, instant paste) that writes `config.toml` and reloads the daemon.
- Details view (full text, full-size image, formats, copy as plain text), rich-text and colour filter chips, pagination with "load more", live refresh while open, desktop notification when instant paste is unavailable.

### CLI
- Shared protocol types from `panora-core`; `--json`, `--kind`, `--pinned`, `--limit`, `--offset`, `copy --paste`, `preview --mime/--out`, `toggle`, `reload`, localized help.

### Packaging
- Review fixes: dialogs no longer lose Escape/Delete/Space to the main window, Super+V toggles through `org.freedesktop.Application.Activate`, the GUI spawned by panod escapes the service sandbox via `systemd-run --user`, package upgrades restart the daemon, `PrivateTmp` dropped so `ydotool` can reach its socket, aspect-correct thumbnails, "copy as plain text" goes through the daemon (`Recall { mime }`), `install.sh` builds as the desktop user and hands display variables to the user session.
- `io.panora.Panora.desktop` with `DBusActivatable=true`, `io.panora.Panora.service`, `.deb` without xclip/wl-clipboard dependencies (`Suggests: wtype, ydotool`), `install.sh` installs rustup when `cargo` is missing, CI builds the package and runs the new X11 integration tests under Xvfb.

## 1.1.0

- Fix silent keyboard, privacy and packaging failures; add the GNOME extension.

## 1.0.0

- First release.
