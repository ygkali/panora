# Releasing Panora

## Versioning

Semantic versioning on the `.deb` version:

- **patch** (`1.3.x`): fixes only; no change to `config.toml`, the IPC
  protocol, the database schema or the on-disk envelope.
- **minor** (`1.x.0`): new features; additive protocol/config changes (older
  clients keep working); schema changes that migrate automatically.
- **major**: anything that breaks an older `panora-cli`/GUI against the
  daemon, or needs manual data migration.

`panora_core::ipc::PROTOCOL_VERSION`, `storage::db::SCHEMA_VERSION` and
`crypto::ENVELOPE_VERSION` are bumped in the same commit as the change that
needs them; `docs/COMPATIBILITY.md` lists what each version accepts.

## Before tagging

1. Every roadmap item for the milestone is done or moved; `docs/ROADMAP.md`
   reflects it.
2. `CHANGELOG.md`: rename *Unreleased* to `[X.Y.Z] - YYYY-MM-DD`, add a fresh
   empty *Unreleased* above it. Keep a Changelog sections: Added / Changed /
   Fixed / Security / Removed.
3. `Cargo.toml` `[workspace.package] version = "X.Y.Z"`, then
   `cargo update --workspace` so `Cargo.lock` follows.
4. `packaging/io.github.ygkali.Panora.metainfo.xml`: add the `<release>`
   entry; `gnome-extension/metadata.json`: bump `version-name` if the
   extension changed.
5. Regenerate `THIRD_PARTY_LICENSES.md` when dependencies changed:
   `cargo about generate about.hbs -o THIRD_PARTY_LICENSES.md`.
6. Green locally: `cargo fmt --all -- --check`, `cargo clippy --workspace
   --all-targets -- -D warnings`, `cargo test --workspace`, the Xvfb X11
   tests, `scripts/keyring-test.sh`, `shellcheck`, `./packaging/build-deb.sh`
   + `lintian --fail-on error,warning dist/*.deb`.
7. **Upgrade path** (`PKG-10`): `scripts/upgrade-test.sh --deb <previous
   release .deb>` (or `--from vX.Y.Z-1`) records entries with the old daemon
   and reopens the same data directory with this build: the entries have to
   come back unchanged, the schema has to reach the version this build
   targets and the pre-migration `history.db.bak-vN` copy has to be there.
   CI runs it against the newest published `.deb`.
8. **Real machine check** (`R-04`): install the package on the target desktop
   (Zorin OS 18 / Ubuntu 24.04 GNOME Wayland, plus one X11 session), run
   `panora-doctor` and `scripts/e2e-test.sh`, try Super+V, recall, instant
   paste, settings. Record the outcome in `docs/verification/`.
9. Push `main` and wait for CI, including the arm64 package job.

## Tagging

```sh
git tag -a vX.Y.Z -m "Panora X.Y.Z"
git push origin vX.Y.Z
```

The `Release` workflow builds amd64 and arm64 packages, the install kit,
`SHA256SUMS` (signed with minisign when the `MINISIGN_KEY` and
`MINISIGN_PASSWORD` secrets exist), an SPDX SBOM, and publishes the GitHub
release with the CHANGELOG section as its notes. A tag containing `-` (for
example `v1.4.0-rc1`) is marked as a pre-release.

`workflow_dispatch` on the same workflow runs the build without publishing;
use it to try the pipeline from a branch.

## APT repository

`.github/workflows/apt-repo.yml` runs when a release is published: it
downloads the release's `.deb` files, adds them to the pool on the
`gh-pages` branch and regenerates the signed `stable` suite with
`packaging/apt-repo.sh` (older versions stay in the pool). GitHub Pages
serves the branch at `https://ygkali.github.io/panora/`, so the repository
lives at `.../panora/apt`.

One-time setup:

1. Create a signing key that is used for nothing else:
   `gpg --quick-generate-key "Panora APT <kompansebuyucu@proton.me>" ed25519 sign 2y`
2. Store it in the repository secrets: `APT_GPG_PRIVATE_KEY` =
   `gpg --armor --export-secret-keys <key id>` and `APT_GPG_PASSPHRASE`.
3. Enable GitHub Pages for the `gh-pages` branch (root folder) in the
   repository settings after the first run created it.
4. Try it locally before relying on it: `packaging/apt-repo.sh /tmp/repo
   dist/panora_*.deb`, then point apt at `file:///tmp/repo` with
   `signed-by=/tmp/repo/panora.gpg`.

Rotate the key before it expires; users fetch the new public key from the
same URL, and the old one keeps verifying the suites signed with it until
then.

## After the release

- Announce (This Week in GNOME, r/gnome, the project Discussions).
- Open the next milestone; move the remaining roadmap items.
- If the release fixed a security issue, publish the GitHub advisory.
