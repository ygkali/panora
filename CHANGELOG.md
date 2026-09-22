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
- `panora-cli watch` (CLI-02): prints one line per history change (`--json`
  for the raw event) by holding a `Subscribe` connection open, instead of
  a caller having to poll `status` in a loop.
- `panora-cli rotate-key` (SEC-01): generates a new master key and reseals
  every stored preview and blob under it, then retires the old one. Safe to
  interrupt at any point — the new key is written to the keyring's pending
  slot before anything is touched, `Database::rekey`/`BlobStore::rekey` are
  each idempotent (a row or file already under the new key is a no-op), and
  the live keyring item is only replaced once both finish — so running
  `rotate-key` again after a crash resumes the same rotation instead of
  starting a new one or losing data. `panora-cli status` reports
  `rotation_incomplete` under health when a previous attempt was left
  unfinished.
- Second-layer lock (SEC-02): `panora-cli lock [set-password|change-
  password|remove-password]` and `panora-cli unlock` gate a *running*
  daemon behind an Argon2id-derived password, independent of the OS
  keyring (which still loads the plain master key unattended, so `panod`
  keeps surviving a reboot with no one there to unlock anything). While
  engaged: `list`/`search` report a count only, `preview` and `copy`/
  `recall` are refused. `privacy.lock_after_idle_minutes` engages it on
  its own after that many minutes of inactivity. Setting a password also
  writes a password-gated backup copy of the live master key to the
  keyring, independent of the daemon's normal unlock flow. Passwords are
  always read from standard input, never a command-line argument.
  `panora-cli status` reports `app_locked`/`lock_password_set`.
- `panora-cli wipe --yes` (SEC-03, panic wipe): hard-deletes the whole
  history — pinned entries included, no undo, unlike `clear` — and retires
  the master key for a freshly generated one, so what was just deleted is
  unrecoverable from the file system too, not only inaccessible through
  Panora. Works regardless of the second-layer lock's state, since a panic
  action has to work under duress, not only after unlocking first.
- `panora-cli export`/`import` (CLI-03): the whole history as one encrypted
  `.panora` archive (tar of `entries.json` + content-addressed blobs,
  sealed with an Argon2id-derived, passphrase-only key — no verifier is
  stored anywhere, so the AEAD tag failing to open on `import` is itself
  the "wrong passphrase" answer). `import` bypasses the privacy gate (the
  archive is the user's own previously-exported data) and deduplicates by
  content hash exactly like a live capture. Both travel over the v3
  protocol directly, so an archive is not limited by the v2 JSON-lines
  response cap the way a `Preview` payload still is. `panora-cli import-
  legacy <copyq|gpaste|clipboard-indicator>` reads another clipboard
  manager's history through the normal privacy-gated `Store` path instead;
  best-effort, since none of the three tools are available to test
  against in this project's CI (only their documented output formats are).
- Fuzzing (SEC-06): `crates/panora-core/fuzz` (`cargo fuzz`), 7 targets —
  IPC decoding (v2 JSON-lines and the v3 frame header, STO-08), the search
  query grammar, the FTS5 `MATCH` expression builder, the storage
  envelope's `open`/`open_with_aad`, the secret/concealed-type MIME flag
  check (ADR 0003), and clipboard content classification. A `fuzz-build`
  CI job compiles every target on each push (a real fuzzing campaign is
  hours, not a CI job's budget); `scripts/fuzz-smoke.sh` runs a short
  timed pass of all of them locally. No crashes found in ~1.8M combined
  executions across an initial smoke run.
- Supply chain hardening (SEC-08): release binaries are built with `cargo
  auditable` (each one carries its own dependency manifest, so `cargo audit
  bin panod` works without the source tree) and with build-machine paths
  stripped (`--remap-path-prefix`). A new `reproducible` release CI job
  rebuilds the same commit on its own runner and compares SHA256 hashes
  against the `build` job's binaries — `publish` only runs once both
  agree — and `scripts/check-reproducible-build.sh` lets anyone repeat the
  same check locally. Verified locally: `panod`/`panora-gui`/`panora-cli`
  are currently byte-identical across independent builds. `PKG-01`
  (1.3.0) already covers `SHA256SUMS` + minisign signing + an SPDX SBOM;
  `cargo vet` is left out as the roadmap itself marks it optional.
- Benchmarks (STO-06): `crates/panora-core/benches/{fts,store,blob}.rs` and
  `crates/panod/benches/ipc.rs` (`cargo bench`, criterion), replacing
  `docs/benchmark.md`'s previous placeholder (B-07). A `bench-build` CI job
  compiles every benchmark on each push; a weekly `benchmark` job actually
  runs them and uploads the results as an artifact. Measured once on this
  session's WSL2 dev machine: FTS5 prefix search over 10,000 rows ~1.1 ms,
  an IPC round trip ~72 µs (both well under target), storing a 1 KiB entry
  ~5.5 ms (slightly over the 5 ms target, inside the 20 ms ceiling — likely
  this VM's disk latency, a bare-metal run should confirm). `panod`'s idle
  RSS and the popup's open time still need a real desktop session (STO-07,
  out of scope here).
- Exclusion list, single source (SEC-10, B-15): `PrivacyConfig::default`'s
  `excluded_apps` used to be a second, independently maintained copy of
  `privacy::DEFAULT_EXCLUDED_APPS` with different contents in each (config.rs
  was missing `org.keepassxc`/`com.bitwarden`/`secrets`) — `PrivacyEngine::
  new` always unions the real one in regardless, so this was a documentation
  drift risk rather than a live gap, but a real one. `config.rs` now builds
  its default straight from `privacy::DEFAULT_EXCLUDED_APPS`, with a
  regression test pinning that down. `sensitive_policy = "mask"` is now
  documented explicitly as a *preview* policy: the real content is still
  stored and still comes back on recall/preview/export, not a way to make a
  secret inaccessible (`drop` is). README/README.tr/docs/book also gained
  the `lock_after_idle_minutes` (SEC-02) config key and the CLI reference
  page gained `watch`/`rotate-key`/`lock`/`unlock`/`wipe`/`export`/`import`/
  `import-legacy`, none of which had made it into the docs yet.
- Public D-Bus API (INT-05): `io.github.ygkali.Panora1` on the session bus
  (`/io/github/ygkali/Panora1`) mirrors the Unix socket for third-party
  integrations (Waybar modules, `gdbus`/`dbus-send` scripts) that would
  rather speak D-Bus — `List`, `Recall`, `Pin`, `Delete`, `Clear`,
  `SetPrivate`, `Status`, and a `Changed(t revision)` signal fed by the same
  `Subscribe` stream CLI-02's `watch` uses. A thin translation layer, not a
  second implementation: every call becomes exactly the request the Unix
  socket already accepts (`panod` is its own IPC client here), so the
  privacy gates, encryption and dedup logic live in exactly one place.
  Payload content, export/import, the lock password and key rotation/wipe
  stay Unix-socket only — the session bus is a broadcast medium other
  processes can watch, a worse place for any of that than a private 0600
  socket. `docs/dbus-api.md`, tested against a real session bus.
- Debian source package (PKG-03): `debian/` at the repository root builds
  through the standard Debian tooling — `dpkg-buildpackage -b`, `sbuild`,
  `pbuilder` — instead of only the hand-rolled `packaging/build-deb.sh`,
  staging the same binaries, systemd user unit, desktop entry, D-Bus service
  file, metainfo, icons, man pages, shell completions and GNOME Shell
  extension. The maintainer scripts restart `panod` for every logged-in user
  the same way the shell script does; a `lintian-overrides` file documents
  why (a per-user systemd unit, not a system one). Verified with a real
  `dpkg-buildpackage -b` and a lintian-clean `.deb`. Getting `panora` into
  the official Debian/Ubuntu archives is a separate, manual process
  (`docs/DISTRIBUTION.md`) this does not automate.
- Property-based tests (QA-07, `proptest`) for `search::fts_expression`
  (`fts_query`), the ciphertext envelope (`Cipher::seal`/`open` and the
  `_with_aad` pair), `percent_decode` and `wanted_order` — run against
  thousands of generated inputs instead of the handful the unit tests
  happened to type. Found and fixed a real bug: a `\0` byte anywhere in a
  search string made every search fail outright. SQLite's FTS5
  query-string parser scans the `MATCH` argument as if it were
  NUL-terminated even though it arrives as length-prefixed TEXT, so the
  embedded NUL truncated the scan mid-quote and SQLite reported
  "unterminated string" instead of the query just not matching that byte —
  `search::fts_expression` now strips `\0` the same way it already strips
  `"`. The crash-free property tests also cover byte-for-byte round-trips
  (envelope, `percent_decode` against a full percent-encoding of arbitrary
  UTF-8) and the `wanted_order` invariants (only known MIME types kept,
  sorted by preference, deduplicated, at most one plain-text flavour,
  idempotent).
- Clear the system clipboard after a recall (CAP-07): `privacy.
  clear_clipboard_after_seconds` (0, the default, disables it). Fires only
  if the clipboard still holds exactly what that recall put there — a
  generation counter bumped on every real clipboard change, not a
  MIME-list comparison, since two different plain-text copies advertise
  the same targets and a comparison that only looked at those would clear
  the wrong one. Two real bugs turned up building this: X11's
  `SetSelectionOwner` needs a round trip (`.check()`) before the release
  is guaranteed to have reached the server, and — caught only by a
  headless-sway run of `scripts/wayland-e2e.sh`, not by the mocked unit
  tests — a deliberate clear looks exactly like a source application
  exiting to `handle_event`, so Wayland's clipboard-persistence feature
  was immediately re-offering the entry the clear had just removed; a
  flag set right before the clear and consumed by the very next
  `OwnerGone` event fixes it without touching persistence's normal
  behaviour for a real exit.
- Recall to PRIMARY for a middle-click paste (CAP-10): `panora-cli copy
  <id> --primary`; `--paste` is ignored with it (there is no keyboard
  shortcut for a PRIMARY paste). `Request::Recall` gained a `to: Selection`
  field, defaulting to `Clipboard` so existing JSON callers are unaffected.
- `history.duplicate_policy` (CAP-08): `bump` (default, unchanged — a
  re-copy of content already in the history moves it to the top with a
  fresh timestamp) or `ignore` (the entry still dedups to the same row,
  never a second one, but its position and timestamp are left alone).
- Fixed a real, pre-existing bug surfaced while building the above:
  `panora-gui`'s `--features fixture` build (used by `scripts/wayland-e2e.
  sh`, `scripts/capture-screenshots.sh` and `scripts/a11y-check.sh`) had
  not actually compiled since SEC-01 — `StatusData`'s `app_locked`/
  `lock_password_set` fields and nine `Request` variants added since then
  (`RotateKey`, `Lock`, `Unlock`, `SetLockPassword`, `Wipe`, `Export`,
  `Import`, `Hello`, `Subscribe`) were never reflected in the fixture,
  because nothing in the regular `cargo build`/`test --workspace` loop
  passes that feature flag; the fixture now answers each of those with an
  explicit "not supported by the GUI fixture" error instead of failing to
  compile, since the popup never sends any of them. `docs/ROADMAP.md`
  §11.4 has the fuller story of how this stayed invisible.
- `panora-cli stats` (CLI-06): counts by kind, pinned/sensitive counts,
  total payload bytes and the oldest/newest entry's time, computed in SQL
  server-side (`Database::stats`) rather than by paging through every
  entry. `--json` gets the same shape as every other command.
- `panora-cli config get/set/validate/edit` (CLI-05): reads and changes
  `config.toml` directly by dotted key path (`history.max_entries`,
  `privacy.sensitive_policy`, ...) without a daemon restart. `get` with no
  key prints the whole file; `set` parses the new value to match the
  existing key's TOML type (boolean, integer, comma-separated list for an
  array) and refuses to write anything that would not pass `validate`;
  `edit` opens `$VISUAL`/`$EDITOR` and reports whether the result is still
  valid afterwards without reverting it. `set` and a valid `edit` both
  best-effort `reload` a running daemon.
- Every `--json` reply is now `{"schema_version": 1, "data": ...}` instead
  of the bare reply (CLI-08), and `panora-cli schema` prints the JSON
  Schema (draft 2020-12) for `data` in every command, keyed by command
  name under `commands` with the underlying type schemas in `$defs`. The
  schemas are generated with `schemars` from the same Rust types that are
  actually serialized (`ResponseData`, `Event`, `Config`, ...), so the
  document cannot drift from the real output the way a hand-written copy
  could. `schema_version` is `panora-cli`'s own output format version,
  independent of the daemon's `protocol` field and the package `version`.
- `GOVERNANCE.md` (DOC-06): who decides what today (one maintainer, with a
  documented path to more as CODEOWNERS grows), where each kind of
  decision is recorded (`docs/ROADMAP.md` for scope, `docs/adr/` for
  architecture), and what happens to the project if the maintainer goes
  quiet for an extended period. Mirrored into the documentation site.
- Locale-aware plural forms and number formatting (I18N-02): the three
  catalogue strings that carry a count (`{n} items`, `{n} items deleted`,
  `{n} items processed.`) now have a matching singular form, picked by a
  new `panora_core::i18n::pluralize`, so English reads "1 item" instead of
  "1 items" (Turkish nouns do not inflect for count, so its singular and
  plural catalogue entries hold the same text on purpose). Byte counts
  (`panora-cli stats`, the popup's storage-usage row, the details view)
  use the language's own decimal separator, a comma rather than a period
  in Turkish. Fixed a real, if narrow, privacy-relevant bug along the way:
  the window-title exclusion list (`privacy.excluded_window_titles`)
  lowercased with Unicode's locale-independent default, under which `İ`
  becomes `i` plus a *combining* dot above rather than a plain `i` — as a
  substring, that never matches plain `i` in the actual window title, so a
  correctly-spelled Turkish exclusion phrase with a capital `İ` could
  silently fail to match at all. Window titles and configured phrases now
  fold through a small Turkish-aware lowercase that also gets the other
  direction right (`I` folds to the dotless `ı`, not `i`).
- The popup remembers its size across restarts (UI-24): resize it once,
  and it opens at that size next time, in a small state file next to
  `config.toml` (window geometry is remembered state, not a preference,
  so it is not in `config.toml` and does not show up in `panora-cli
  config get`). Fixed a real multi-monitor bug found while building this:
  the X11 "open next to the pointer" placement clamped to the *combined*
  virtual screen across every monitor rather than the one the pointer is
  actually on, so on a multi-monitor desktop the popup could spill from
  the pointer's monitor onto a neighbouring one; it now asks RandR which
  monitor the pointer is on and clamps to that one instead.

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
