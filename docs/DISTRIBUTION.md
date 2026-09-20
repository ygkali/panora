# How Panora is distributed

Which channels are official, which distributions are tested, how long a
release is supported, and what a packager needs to know. `docs/RELEASING.md`
covers the mechanics of cutting a release; this file is the policy around it.

## Official channels

Only these carry builds made by the project. Everything else is a community
package (see below).

| Channel | What it is | Signed by | Updates |
|---|---|---|---|
| [GitHub Releases](https://github.com/ygkali/panora/releases) | `panora_<version>_<arch>.deb` for amd64 and arm64, the install kit, `SHA256SUMS`, an SPDX SBOM | `SHA256SUMS.minisig` (minisign) | manual |
| APT repository — `https://ygkali.github.io/panora/apt` | the same `.deb` files in a `stable main` suite | the repository key, `InRelease` (GPG) | `apt upgrade` |
| Source | this git repository; `./packaging/build-deb.sh` or `cargo build --release` | git tags | `git pull` |

The install kit (`panora-<version>-install-kit.tar.gz`) is the `.deb` plus
`install.sh`, `uninstall.sh` and `test-local.sh` for people who would rather
run one script than read the README.

There is no Flatpak and no Snap. Panora needs the session bus, the Secret
Service, the X11 or Wayland clipboard protocols and — on older GNOME — a
Shell extension; the portal story for all of that is not there yet. The
trade-offs are recorded as `PKG-04` in `docs/ROADMAP.md`.

### Verifying a download

```sh
sha256sum -c SHA256SUMS --ignore-missing
minisign -Vm SHA256SUMS -P <the public key from the release notes>
```

The APT repository verifies itself: `apt` checks `InRelease` against the key
in `/usr/share/keyrings/panora.gpg`, and the `Packages` hashes against the
`.deb`.

## Supported distributions

Panora needs **GTK 4.12** and **libadwaita 1.5**. That is the whole rule; it
happens to draw the line at the 2024 distribution generation.

| Distribution | Status | Notes |
|---|---|---|
| Zorin OS 18 | primary target | the desktop the release is verified on (`R-04`) |
| Ubuntu 24.04 LTS | tested | GNOME 46; the extension carries the `zorin` session mode too |
| Ubuntu 25.10 | tested | GNOME 49; native `ext-data-control`, no extension needed for capture |
| Debian 13 (trixie) | tested | GNOME 48 |
| Linux Mint 22 | best effort | Cinnamon: X11 backend, no Shell extension |
| Pop!_OS 24.04 | best effort | COSMIC: `wlr-data-control`, bind the shortcut yourself |
| Fedora, Arch, openSUSE, NixOS | community | see the packaging starting points below |
| Ubuntu 22.04, Zorin OS 17, Mint 21, Debian 12 | **not supported** | GTK too old; the `.deb` refuses to install |

"Tested" means CI builds and installs the package there, or a maintainer ran
`scripts/e2e-test.sh` on it for the release. "Best effort" means it is
expected to work and bug reports are welcome, but nothing checks it per
release. `docs/DESKTOPS.md` has the per-desktop feature matrix.

## Versioning and support window

Semantic versioning, as in `docs/RELEASING.md`:

- **Patch** (`1.3.x`) — fixes only. No schema change, no config change, no
  IPC protocol change.
- **Minor** (`1.x.0`) — features. The history database may gain a schema
  version; the migration runs on first start and keeps a
  `history.db.bak-vN` copy next to the file. Config keys are added, never
  removed without a release of warning. The IPC protocol number may rise;
  the daemon keeps answering the previous one.
- **Major** — a break that cannot be migrated. Not planned.

Only the newest release is supported. When `1.4.0` is out, `1.3.x` gets no
more fixes; security fixes for the previous minor are the exception and are
published as a patch on it.

Upgrades are tested, not assumed: `scripts/upgrade-test.sh` runs the
previous release's daemon against a scratch history, then opens the same
directory with the new build and checks the entries, the search index, the
migrated schema and the backup. CI runs it against the newest published
`.deb` (`PKG-10`).

Downgrades are not supported. A newer schema makes the daemon refuse the
file rather than damage it; restore the `history.db.bak-vN` copy to go back.

## Community packages

Packaging Panora for a distribution is welcome and does not need permission
— it is GPL-3.0-only. Two requests:

1. **Do not call it official.** Link to this repository as the upstream and
   say who built the package. Bug reports that turn out to be packaging bugs
   are closed upstream with a pointer to the packager.
2. **Do not patch out the security work.** The systemd unit hardening, the
   Secret Service master key, `PR_SET_DUMPABLE=0`, the 0700 data directory
   and the extension's session-bus-only policy are load bearing.
   `docs/security-checklist.md` says why each one is there.

Starting points live in `packaging/` and are **contributed, untested by the
project**: `packaging/aur/PKGBUILD`, `packaging/nix/flake.nix`,
`packaging/rpm/panora.spec`. They are kept in the tree so a packager does not
start from nothing, not because the project builds them. If you get one
working, a pull request that corrects it is the best possible contribution.

If a package lands in a distribution's own archive, open an issue so the
README can link to it and so release notes can mention the lag.

## Notes for packagers

- **Binaries:** `panod` (user daemon), `panora-gui` (the popup, also
  installed as `panora`), `panora-cli`, `panora-doctor` (a shell script).
- **Build:** Rust 1.92 or newer, `pkg-config`, `libgtk-4-dev` ≥ 4.12,
  `libadwaita-1-dev` ≥ 1.5. `cargo build --release --workspace`, or
  `packaging/build-deb.sh` to see every install path in one place.
- **Runtime:** GTK 4, libadwaita, an icon theme, and a Secret Service
  provider (`gnome-keyring` or `kwallet` with the Secret Service interface).
  `xclip`/`wl-clipboard` are only needed by the test scripts, `wtype` or
  `ydotool` only for instant paste on Wayland outside GNOME.
- **Services:** `panod.service` is a **user** unit
  (`/usr/lib/systemd/user/`), never a system one; it must not be enabled by
  default with a system preset. `io.github.ygkali.Panora.service` is the
  D-Bus activation file, also session scope.
- **Files:** desktop entry, AppStream metainfo, four man pages, the icon in
  `hicolor`, and `gnome-extension/` into
  `/usr/share/gnome-shell/extensions/panora@ygkali.github.io/` with the
  compiled gschema. `glib-compile-schemas` must run for the extension's
  settings.
- **Nothing writes outside `$HOME`.** The history, the config and the socket
  are per user (`~/.local/share/panora`, `~/.config/panora`,
  `$XDG_RUNTIME_DIR`). There is no system state to migrate or purge.
- **No network.** The daemon and the extension open no sockets beyond the
  session bus and the local IPC socket; CI asserts that
  (`scripts/security-check.sh`, the extension security job). A build that
  needs the network is a bug.
- **Reproducibility:** `Cargo.lock` is committed and CI builds with
  `--locked`. Vendoring with `cargo vendor` works offline.

Questions that are not answered here belong in a GitHub issue; packaging
questions get answered.
