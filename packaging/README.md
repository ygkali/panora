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

"Untested" is literal: no maintainer has run `makepkg`, `nix build` or
`rpmbuild` on these files, they are not in CI, and the versions in them go
stale between releases. They record the install layout and the dependency
set so a packager can see what Panora expects, and they are the place to
send a correction.

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
