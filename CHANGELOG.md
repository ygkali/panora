# Changelog

All notable changes to Panora are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1/) and the project uses
semantic versioning (see `docs/RELEASING.md`).

## [Unreleased]

Everything below ships as **1.3.0**, the first public release, once the
real-machine verification in `docs/RELEASING.md` is done.

### Added
- Popup: the panel closes when focus moves to another window (Win+V
  behaviour, `ui.close_on_focus_loss`), `Shift+Enter` puts only the plain
  text on the clipboard, `Ctrl+1`…`Ctrl+9` pick the first nine rows (which
  show their number), `Home`/`End`/`PgUp`/`PgDn` jump in the list, typing
  anywhere starts a search, and deleting shows a toast with **Undo**.
- Undo for deletions: a deleted entry keeps its tombstone and blobs for 30
  seconds; `panora-cli restore <id>` and the toast bring it back. Retention
  evictions and *clear* stay final.
- `panora-cli store [FILE]` records text from a file or standard input as a
  new entry (through the same privacy gate) and puts it on the clipboard;
  `panora-cli pick` and `list/search --format TEMPLATE` print one line per
  entry for dmenu/rofi/fuzzel style pickers.
- Clipboard persistence on Wayland: when the source application exits on
  a compositor that drops the selection (Sway, Hyprland, other wlroots
  desktops), panod re-offers the entry it just recorded, as it always did
  on X11. Mutter and KWin are detected from their globals and left alone;
  `history.persist_on_wayland = auto | always | never` overrides.
- Image entries carry a PNG thumbnail (longest side 320 px, made on
  capture or on first request) so the popup no longer transfers and decodes
  full images per row; thumbnails are never offered on the clipboard.
- `scripts/wayland-e2e.sh` runs panod against a headless sway compositor
  with a throwaway keyring and checks capture, recall, HTML, private mode,
  persistence and store/restore through wl-copy/wl-paste; CI runs it.
- Instant paste sends `Ctrl+Shift+V` when the focused window is a terminal
  emulator (X11 and the GNOME extension; `panora_core::apps` knows the
  common ones).
- Content filters under `[privacy]`: `min_text_length`,
  `ignore_whitespace_only`, `ignore_patterns` (regular expressions matched
  against the trimmed text, at most 32 of them) and `capture_kinds` decide
  what is recorded once the text is known; they also guard `panora-cli
  store` and the GNOME bridge.
- Search covers the whole text, not only the 500-character preview: text
  entries are indexed up to 64 K characters (`history.index_full_text`,
  on by default) and the index folds case and diacritics, so `istanbul`
  finds `İstanbul`. Existing databases are migrated to schema 3 with a
  backup next to them; older entries stay searchable by their preview.
- `panora-doctor --report[=FILE]` writes a diagnostic file for bug reports:
  the checks, versions, session variables, daemon status, unit state, the
  panod journal, the extension state and the configuration, with the home
  directory, user name and host name replaced, `ignore_patterns` left out
  and no clipboard content.
- `Status` carries `health` findings with stable codes. The first one,
  `extension_missing`, is set when capture depends on the GNOME Shell
  extension and it is not on the bus: the popup shows a banner with an
  **Enable** button that runs `gnome-extensions enable`, and
  `panora-cli status` prints the finding.
- Secrets are recognised and kept short-lived: private key blocks, JWTs,
  tokens with vendor prefixes (GitHub, AWS, Slack, Stripe, Google, ...),
  card numbers (Luhn), IBANs (mod 97) and single high-entropy tokens.
  `privacy.sensitive_policy` decides: `mask` (default) records the entry
  behind a masked preview and outside the search index, `drop` never
  records it, `store` records it as it is. Flagged entries show a lock,
  are still recallable, and are removed after `sensitive_ttl_minutes`
  (10; pinned entries stay). `panora-cli list` marks them with `!` and
  `--format` gets `{sensitive}`. The database moves to schema 4.
- A welcome on first run: three pages on the shortcut, the privacy model
  and what this session can do (from the daemon's capabilities). A
  `first-run` file next to `config.toml` records that it was seen.
- Settings: a **System** group with a switch for starting panod with the
  session (`systemctl --user enable/disable panod.service`) and the space
  the history takes on disk.
- `history.max_total_bytes` (512 MiB) caps the payload bytes kept: the
  oldest unpinned entries go first, pinned entries count but stay.
- Hourly upkeep now also runs `PRAGMA optimize`, checkpoints the WAL,
  vacuums the database once a quarter of it is free pages, and removes
  blobs no row references (left by a crash between writing a blob and
  recording it) along with stale temporary files.
- Search grammar in the popup and the CLI: `"quoted phrases"`, `kind:`,
  `app:`, `pinned:`, `before:`/`after:` (a day or `7d`-style spans) and
  `re:` for a regular expression over the preview and the indexed text.
  Matches are shown in bold in the popup.
- The details view acts on what the entry is: a link opens in the browser
  or shows as a QR code for a phone, a colour copies as hex, `rgb()` or
  `hsl()`, a copied file list opens its folder, and an image saves to a
  file and shows its pixel size.
- `privacy.excluded_window_titles`: phrases that keep a copy out of the
  history while the focused window's title contains one (for banking
  tabs and the like). X11 reads `_NET_WM_NAME`; the GNOME extension sends
  the title with a new `PushManyFrom` call and falls back to `PushMany`
  on an older daemon. The title is judged and dropped, never stored.
- `history.max_images` (200) caps the image entries kept.
- The popup no longer rebuilds its list when a revision bump changed
  nothing visible.
- The popup opens where the pointer is: on X11 the window is moved next to
  the pointer once mapped, on GNOME the Shell extension moves it after
  activation (`move-to-pointer` setting), and on Sway, Hyprland and other
  wlroots compositors it becomes a layer-shell overlay anchored to a
  corner (`ui.layer_anchor`) when the popup is built with the
  `layer-shell` feature (the library must be linked in, and Ubuntu 24.04
  does not ship it, so the .deb leaves it out). `ui.position` picks
  pointer or centre.
- On wlroots compositors and KWin the source application of a copy is
  now known (`wlr-foreign-toplevel-management`): the excluded-application
  and window-title lists work there, and `app:` searches match.
- The GNOME Shell extension has a preferences page
  (`gnome-extensions prefs panora@ygkali.github.io`): the Super+V shortcut
  can be recorded by pressing it instead of by editing dconf, and the
  *open next to the pointer* switch is there too.
- `docs/launch.md` holds the announcement drafts for the first release --
  Show HN, r/linux, r/gnome, GNOME Discourse, Fosstodon, This Week in
  GNOME and the Turkish forums -- each one leading with what Panora does
  not do as well as what it does, plus the order to post them in.
- A documentation site built with mdBook (`docs/book/`, published to
  https://ygkali.github.io/panora/docs/): the user guide, the popup
  shortcuts, the search syntax, a complete `config.toml` reference, the CLI
  reference and the privacy model, with search across all of it. The
  desktop matrix, troubleshooting, distribution policy, contributing guide
  and changelog are included from their files rather than copied.
- The interface catalogue moved out of `i18n.rs` and into gettext files:
  `po/panora.pot` is the template a translator starts from and `po/tr.po`
  the Turkish catalogue. A build script turns them into the same `Strings`
  struct as before -- no gettext runtime, no behaviour change -- and refuses
  a catalogue with a missing, empty or stale entry.
- `docs/DISTRIBUTION.md`: which channels are official, which distributions
  are tested and which are best effort, what the support window is, and what
  a packager has to keep (the user unit, the compiled gschema, the
  hardening). Linked from both READMEs.
- Packaging starting points for other distributions, marked untested and
  outside CI: `packaging/aur/PKGBUILD` (+ `.SRCINFO`),
  `packaging/nix/flake.nix`, `packaging/rpm/panora.spec`, with
  `packaging/README.md` saying what is supported and what is not.
- `scripts/upgrade-test.sh` proves the upgrade path: the previous release
  writes a history in a scratch directory, this build reopens it and the
  entries, the search index, the migrated schema version and the
  pre-migration backup are all checked. CI runs it against the newest
  published `.deb`.
- The GNOME Shell extension is linted: `gnome-extension/eslint.config.mjs`
  declares the GJS globals and forbids `eval`, implicit globals and
  synchronous subprocesses in the Shell; CI runs ESLint over the extension.
- A signed APT repository on GitHub Pages: publishing a release adds its
  `.deb` files to `https://ygkali.github.io/panora/apt` (suite `stable`,
  amd64 and arm64), so updates arrive with `apt upgrade`.
  `packaging/apt-repo.sh` builds the suite; `docs/RELEASING.md` has the
  key setup.
- Recording pauses while the session is locked (`org.gnome.ScreenSaver` /
  `org.freedesktop.ScreenSaver` `ActiveChanged`); `panora-cli status` shows
  `locked`.
- panod marks itself non-dumpable (`PR_SET_DUMPABLE=0`) and the unit adds
  `LimitCORE=0`, an empty capability set, `ProtectProc=invisible`,
  `PrivateDevices`, `ProtectClock`, `ProtectHostname`, `UMask=0077` and a
  `@system-service` syscall filter; `systemd-analyze security` scores it
  1.8 (was 4.x). A test asserts that no clipboard content ever reaches the
  daemon log.
- Application icon (scalable and symbolic), AppStream metadata, man pages
  for every binary and bash/zsh/fish completions, all installed by the
  `.deb`; the package also carries a DEP-5 `copyright`, a Debian-format
  changelog, `THIRD_PARTY_LICENSES.md` and md5sums, and is lintian-clean.
- `--version` for `panora-cli`, `panora-gui` and `panod`; `panod --help`.
- Schema migration framework for the history database: the stored schema
  version is checked on every start, a newer schema is refused, an older one
  is backed up with `VACUUM INTO` and migrated step by step, and a damaged
  file fails `PRAGMA quick_check` instead of half-working.
- Key fingerprint in the database: a history opened with a different master
  key (a reset keyring) is refused with a clear message instead of showing
  `[decryption failed]` rows.
- Integration tests for the IPC server on a real Unix socket (round trips,
  frame and request limits, peer UID check) and for the Secret Service round
  trip through a throwaway gnome-keyring (`scripts/keyring-test.sh`).
- CI: MSRV job (Rust 1.85), `cargo doc` without warnings, shellcheck, keyring
  job, desktop/AppStream validation, lintian, an install smoke test of the
  package, and an arm64 package next to amd64. A tag-triggered release
  workflow builds both packages and the install kit, writes `SHA256SUMS`
  (minisign-signed when a key is configured) and an SPDX SBOM, and publishes
  the GitHub release with the matching changelog section.
- `uninstall.sh --yes` and `--purge-data`; `scripts/capture-screenshots.sh`
  renders the README screenshots from the fixture data under Xvfb.
- English `README.md`, `docs/TROUBLESHOOTING.md`, `docs/RELEASING.md`,
  `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, issue and pull
  request templates, Dependabot, `REUSE.toml`, `AUTHORS.md`, and
  `docs/ROADMAP.md` with every planned task identified.
- IPC protocol v3 (STO-08): a binary framing (`u32` length + JSON header +
  raw payload) replaces base64-in-JSON for anything large — payloads over
  8 KiB travel as a `memfd_create` file descriptor passed over the socket
  with `SCM_RIGHTS` instead of being inflated 4/3 and squeezed under the
  64 MiB reply cap. `Hello` negotiates the protocol version and `Subscribe`
  turns a connection into a push stream of `{event:"changed",revision}`
  frames, so a client can wait for changes instead of polling `Status`.
  v2 JSON-lines clients are still served on the same socket (the daemon
  tells them apart by peeking the connection's first byte).

### Changed
- The popup no longer waits on the daemon: history pages, previews, image
  thumbnails and the details view are fetched and decoded on a worker
  thread and land on the GTK loop when ready, so typing and scrolling stay
  smooth with large images in the list. A page that arrives after the
  query changed is dropped.
- Dependencies: gtk4-rs 0.11 / libadwaita-rs 0.9 (same GTK and libadwaita
  runtime requirements), rusqlite 0.40 with bundled SQLite 3.53 and toml 1.
- **Identifiers moved to the GitHub namespace.** Application id
  `io.panora.Panora` → `io.github.ygkali.Panora`, bus names
  `io.panora.GnomeBridge1` / `io.panora.GnomeShell1` →
  `io.github.ygkali.Panora.GnomeBridge1` / `io.github.ygkali.Panora.GnomeShell1`,
  extension UUID `panora@panora-clipboard.org` → `panora@ygkali.github.io`,
  package maintainer `ygkali <kompansebuyucu@proton.me>`. The old names used
  domains the project does not own. Upgrading from 1.2.0 removes the old
  extension directory; enable the new UUID once after logging in again.
- The workspace moved from `panora/` to the repository root: `cargo install
  --git` works and CI no longer needs working-directory overrides.
  `KUR.sh`, `TEST.sh` and `KALDIR.sh` stay as Turkish-named wrappers of
  `install.sh`, `test-local.sh` and `uninstall.sh`.
- `panora-cli` is built on clap: `panora-cli <command> --help` for every
  command, `completions`, `man`, and documented exit statuses (1 daemon
  error, 2 usage, 3 daemon not running, 4 no such entry). `list`, `search`,
  `copy`/`recall`, `preview`/`show`, `pin`, `unpin`, `delete`/`rm`, `clear`,
  `private on|off`, `status`, `toggle`, `reload` and `--json` work as before;
  `status` now also prints `needs_bridge`.
- `install.sh`, `uninstall.sh`, `test-local.sh` and `panora-doctor` speak
  English by default and Turkish when the locale is Turkish
  (`LANG`/`LC_ALL`/`PANORA_LANG`); the doctor's status labels are
  `OK/WARN/ERROR/INFO` and its JSON keys never change. `scripts/e2e-test.sh`
  is English only.
- The popup is a **single-column Win+V style panel** (420×660, minimum
  340×420): the newest entry is the top of a list, Up/Down move one row at a
  time, the 9 px type badge is gone in favour of a type icon + age + size +
  source line under the content, row actions (pin, details, delete) appear
  on hover, keyboard focus or selection while pinned rows always show their
  star, **clear history** moved to the header bar and the filter chips wrap.
- Accessibility: every icon button and history row has a screen-reader
  name, row actions are 28 px (WCAG 2.2 SC 2.5.8), fixed pixel font sizes are
  gone so typography follows the user's text scale, secondary text contrast
  rose from 0.5 to 0.7 alpha (SC 1.4.11), keyboard focus draws its own ring
  (SC 2.4.7). RTL: alignments use `halign: Start`, so the layout mirrors.
- The `max_mime_bytes` ceiling is 40 MiB (was 256 MiB): anything larger
  could be stored but never previewed or exported through the 64 MiB IPC
  reply cap.
- Database schema version 2 adds `deleted_at`; existing files are backed up
  (`history.db.bak-v1`) and migrated on the first start.
- The minimum supported Rust is 1.92 (what the dependency tree needs);
  `install.sh` installs rustup when the system toolchain is older.
- The Debian package `Recommends: gnome-keyring | kwalletmanager` and
  `Suggests: wtype, ydotool, xclip, wl-clipboard`.
- Docs: ADR 0001 and 0002 carry addenda for what was never built
  (gtk4-layer-shell, the `org.panora.Pano1` D-Bus API); `docs/benchmark.md`
  is a goals-and-method note until the benches exist; 1.0/1.1-era reports
  and test artefacts describing the abandoned arboard/wl-clipboard design
  were removed.

### Fixed
- Switching `history.record_primary` in the settings needed a daemon
  restart; the capture loop now opens or drops the PRIMARY watch on reload.
- The GNOME bridge validates its caller: `Push`/`PushMany` are accepted only
  from the owner of `org.gnome.Shell`, so another session-bus process (a
  Flatpak with only `--socket=session-bus`, for instance) can no longer
  inject fabricated entries. The bus signature is unchanged.
- New backend capability `source_app`: `panora-cli status` shows it and the
  settings dialog says on plain Wayland sessions that the exclusion list
  cannot fire there (the data-control protocols expose no client identity).
- `Cargo.toml`, `panod.service` and the package `Homepage` point at the real
  repository.

## [1.2.0] - 2026-09-11

### Zorin OS 18 / Ubuntu 24.04
- `install.sh` refuses releases older than Ubuntu 24.04 / Debian 13 / Zorin
  18 with a clear message, detects a too-old apt `cargo` (1.75) and installs
  rustup instead, picks the `libglib2.0-0t64` runtime name, installs
  `libglib2.0-bin` (where `glib-compile-schemas` really lives) and tolerates a
  broken third-party apt repo.
- GNOME 46 Wayland bridge: `PushMany` carries text + HTML (or uri-list) per
  change so bridge entries have the same fidelity as native captures; the
  daemon recognises the echo of its own recall; a failed bridge service is
  fatal when capture depends on it; backend detection no longer calls the
  blocking zbus API inside the runtime (this crashed panod at startup on
  sessions without `XDG_CURRENT_DESKTOP`).
- Extension: loads in Zorin's `zorin` session mode, takes `<Super>v` away
  from GNOME's notification list while enabled (restored on disable), waits
  for focus to leave the popup before pasting, uses evdev key codes so Ctrl+V
  works on any layout, activates the popup with a 25 s timeout and only falls
  back to spawning when no D-Bus service exists.
- New `panora-doctor` (installed to /usr/bin) diagnoses the session, daemon,
  extension, D-Bus names, keyring and shortcut conflicts;
  `scripts/e2e-test.sh` runs a PASS/FAIL functional test of every feature on
  the real machine; `test-local.sh` runs the doctor first.

### Daemon
- Native X11 backend (x11rb): XFIXES change events instead of 180 ms polling,
  `ConvertSelection` reads with INCR, panod becomes the selection owner on
  recall and serves every stored format (text + HTML, images) with INCR for
  large payloads, XTEST instant paste, re-offer of the last entry when the
  owning application exits. `xclip` is no longer required.
- Native Wayland backend (wayland-client): `ext-data-control-v1` and
  `wlr-data-control-v1`, event-driven capture with the MIME list delivered
  before any payload, multi-format data sources on recall, primary selection
  support. `wl-clipboard` is no longer required. Consecutive copies of the
  same type are no longer missed.
- GNOME bridge backend for GNOME ≤ 47 (no data-control): recall and paste
  through the Shell extension's helper service; bridge pushes are ignored
  when a native backend is active so entries are never duplicated.
- Live configuration reload (`ReloadConfig`), preserving private mode;
  retention runs hourly and on every store; evicted or deleted entries
  release their encrypted blobs unless another entry still references them;
  revived entries are indexed for search again.
- `Recall { paste }` synthesizes Ctrl+V after the clipboard is set; `Toggle`
  activates the GUI over D-Bus; `Status` reports revision, version, protocol
  and capabilities; SIGTERM shuts down cleanly.
- FTS5 prefix search (`mer` finds `merhaba`), safe against operator
  injection.

### GUI
- Unique application: Super+V / `panora` / `panora-cli toggle` toggle the
  popup; D-Bus activation file installed.
- Turkish and English catalogues (`ui.language`), light/dark/system theme
  (`ui.theme`).
- Settings dialog (history limits, PRIMARY recording, private start,
  excluded applications, language, theme, instant paste) that writes
  `config.toml` and reloads the daemon.
- Details view (full text, full-size image, formats, copy as plain text),
  rich-text and colour filter chips, pagination with "load more", live
  refresh while open, desktop notification when instant paste is
  unavailable.

### CLI
- Shared protocol types from `panora-core`; `--json`, `--kind`, `--pinned`,
  `--limit`, `--offset`, `copy --paste`, `preview --mime/--out`, `toggle`,
  `reload`, localized help.

### Packaging
- Review fixes: dialogs no longer lose Escape/Delete/Space to the main
  window, Super+V toggles through `org.freedesktop.Application.Activate`, the
  GUI spawned by panod escapes the service sandbox via `systemd-run --user`,
  package upgrades restart the daemon, `PrivateTmp` dropped so `ydotool` can
  reach its socket, aspect-correct thumbnails, "copy as plain text" goes
  through the daemon (`Recall { mime }`), `install.sh` builds as the desktop
  user and hands display variables to the user session.
- Desktop entry with `DBusActivatable=true`, D-Bus service file, `.deb`
  without xclip/wl-clipboard dependencies (`Suggests: wtype, ydotool`),
  `install.sh` installs rustup when `cargo` is missing, CI builds the package
  and runs the new X11 integration tests under Xvfb.

## [1.1.0] - 2026-08-25

- Fix silent keyboard, privacy and packaging failures; add the GNOME
  extension.

## [1.0.0] - 2026-08-18

- First release.

[Unreleased]: https://github.com/ygkali/panora/compare/v1.2.0...HEAD
[1.2.0]: https://github.com/ygkali/panora/releases/tag/v1.2.0
[1.1.0]: https://github.com/ygkali/panora/releases/tag/v1.1.0
[1.0.0]: https://github.com/ygkali/panora/releases/tag/v1.0.0
