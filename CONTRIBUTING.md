# Contributing to Panora

Thanks for helping. This page tells you how the project is built, tested and
reviewed so a contribution lands on the first try.

## Where things are

| Path | What |
|---|---|
| `crates/panora-core` | Model, privacy engine, config, encrypted storage, IPC protocol, i18n |
| `crates/panod` | The daemon: X11 / Wayland / GNOME bridge backends, keyring, D-Bus, IPC server |
| `crates/panora-gui` | GTK4/libadwaita popup, details view, settings |
| `crates/panora-cli` | Command-line client (clap) |
| `gnome-extension` | GNOME Shell extension: Super+V, clipboard bridge for GNOME ≤ 47, paste helper |
| `packaging` | `.deb` build, systemd unit, desktop/D-Bus/AppStream files, icons, man pages |
| `scripts` | `panora-doctor`, end-to-end test, keyring test, security checks |
| `docs` | Roadmap (`docs/ROADMAP.md`, every task has an id), ADRs, protocol matrix, security notes |

Pick work from `docs/ROADMAP.md` and mention the id (for example `UI-03`) in
the issue or pull request title.

## Building

Debian 13, Ubuntu 24.04 or newer; Rust 1.92+ (rustup); GTK 4.12 and
libadwaita 1.5 development packages:

```sh
sudo apt install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
  xvfb dbus-x11 xclip wl-clipboard gnome-keyring shellcheck lintian
cargo build --workspace
```

The popup can run without a daemon for UI work:

```sh
cargo run -p panora-gui --features fixture
```

## Tests

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                                   # headless, X11 tests self-skip
Xvfb :99 -screen 0 1280x800x24 -ac & DISPLAY=:99 dbus-run-session -- \
  cargo test -p panod --test x11_integration -- --test-threads=1
scripts/keyring-test.sh                                  # Secret Service round trip
shellcheck -S warning install.sh uninstall.sh test-local.sh packaging/*.sh scripts/*.sh scripts/panora-doctor
```

On a real desktop, after `./install.sh`: `panora-doctor`, then
`scripts/e2e-test.sh --safe`. CI runs all of the above plus cargo-audit,
cargo-deny, the extension checks and a lintian-clean package build for amd64
and arm64.

## Ground rules

- **Privacy first.** Nothing may read a clipboard payload before the privacy
  engine has judged the offered MIME list (ADR 0003). Payloads never appear
  in logs, error messages or the session bus in the clear.
- **No network.** The workspace has no network dependency and
  `scripts/security-check.sh` verifies it. Anything that needs the network
  belongs to the future sync module, behind its own package.
- **No `unsafe`.** All crates carry `#![forbid(unsafe_code)]`.
- **Tests come with the change.** A bug fix adds the test that would have
  caught it; a feature adds unit tests and, where it touches the desktop, an
  e2e step.
- **Small commits, present tense subjects**, English, no trailer lines. The
  body explains why, not what.
- **Code and comments are English.** User-facing strings live in
  `crates/panora-core/src/i18n.rs` (Turkish and English) or the scripts'
  translation tables.
- Keep `docs/ROADMAP.md` honest: tick off what you finish, add what you find.

## Pull requests

1. Fork, branch from `main`, keep the branch focused on one roadmap item.
2. Run the checks above; CI must be green.
3. Describe the user-visible change in `CHANGELOG.md` under *Unreleased*.
4. One maintainer review is required; expect questions about privacy
   implications first and code style second.

By contributing you agree that your work is released under GPL-3.0-only, the
project licence.
