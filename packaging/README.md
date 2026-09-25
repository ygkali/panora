# packaging/

The Debian package is the one the project builds, tests and ships.
Everything else here is a starting point for someone packaging Panora
elsewhere.

| Path | Status |
|---|---|
| `build-deb.sh`, `copyright`, `panod.service`, `io.github.ygkali.Panora.*`, `icons/`, `man/` | **supported** — CI builds, `lintian`-checks and installs the `.deb` on every push |
| `make-kit.sh` | **supported** — the install kit published with each release |
| `apt-repo.sh` | **supported** — builds the signed APT suite the release workflow publishes |
| `aur/PKGBUILD`, `aur/.SRCINFO` | **untested**, contributed as a starting point |
| `nix/flake.nix` | **untested**, contributed as a starting point |
| `rpm/panora.spec` | **untested**, contributed as a starting point |
| Flatpak | **investigated, not attempted** — see below |

"Untested" is literal: no maintainer has run `makepkg`, `nix build` or
`rpmbuild` on these files, they are not in CI, and the versions in them go
stale between releases. They record the install layout and the dependency
set so a packager can see what Panora expects, and they are the place to
send a correction.

## Why there is no Flatpak manifest here

A Flatpak build was researched (`docs/ROADMAP.md` §11.5, PKG-04) rather
than attempted, because the answer to the one question that matters came
back negative before a manifest was worth writing: on wlroots compositors
(Sway, Hyprland, Niri) the compositor does not expose
`wlr-data-control`/`ext-data-control-v1` to a sandboxed client at all —
not a Flatpak permission that can be granted, a compositor-side denial —
which is exactly the protocol family `panod`'s native Wayland backend
depends on (ADR 0001). CopyQ, which uses the same "GNOME Shell extension
bridge" design Panora uses for GNOME ≤ 47, documents the matching failure
for that path: the extension cannot register with GNOME Shell from inside
a sandbox. A Flatpak build would work, at best, on X11 and KDE/KWin, which
is a real step down from "every Wayland compositor" and not a trade a
clipboard manager should make quietly. Revisit if a compositor or Flatpak
itself ever adds an explicit grant for these protocols; none exists today.

Three things the Debian package gets right and a port must not lose:

- `panod.service` is a **systemd user unit**. A system unit would run the
  daemon as root against the wrong session bus and the wrong keyring.
- The GNOME Shell extension's gschema has to be compiled
  (`glib-compile-schemas`) or the extension will not load.
- The hardening in `panod.service` and the Secret Service master key are
  part of the threat model, not decoration. `docs/security-checklist.md`
  says what each line defends against.

`docs/DISTRIBUTION.md` has the full policy: official channels, the tested
distribution list, the support window and the rest of the notes for
packagers.
