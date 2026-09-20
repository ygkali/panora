#!/usr/bin/env bash
# Copyright (C) 2026 Panora contributors
# SPDX-License-Identifier: GPL-3.0-only
#
# Build (or update) the signed APT repository that GitHub Pages serves:
#
#   packaging/apt-repo.sh REPO_DIR DEB...
#
# REPO_DIR is the `apt/` directory of the gh-pages checkout (created when
# missing). Every .deb given is copied into pool/main/p/panora/, older
# versions already in the pool are kept, and the indexes for the `stable`
# suite are regenerated for every architecture found:
#
#   dists/stable/main/binary-<arch>/Packages{,.gz}
#   dists/stable/{Release,Release.gpg,InRelease}
#   panora.gpg                      (the public key, binary format)
#
# Signing needs a GPG secret key in the default keyring; APT_GPG_KEY_ID
# picks it (default: the first secret key) and APT_GPG_PASSPHRASE unlocks
# it when set. With no secret key at all the indexes are written unsigned,
# which is only useful for a local look.
set -euo pipefail

REPO="${1:?usage: apt-repo.sh REPO_DIR DEB...}"
shift
[ "$#" -gt 0 ] || { echo "apt-repo.sh: no .deb given" >&2; exit 2; }

for tool in apt-ftparchive gpg dpkg-deb; do
  command -v "$tool" >/dev/null || { echo "apt-repo.sh: $tool is missing (apt-utils, gnupg, dpkg)" >&2; exit 2; }
done

SUITE="stable"
COMPONENT="main"
POOL="$REPO/pool/$COMPONENT/p/panora"
DISTS="$REPO/dists/$SUITE"
install -d "$POOL" "$DISTS"

for deb in "$@"; do
  [ -f "$deb" ] || { echo "apt-repo.sh: $deb is not a file" >&2; exit 2; }
  dpkg-deb --info "$deb" >/dev/null
  install -m 0644 "$deb" "$POOL/$(basename "$deb")"
done

# One Packages index per architecture present in the pool.
archs="$(for f in "$POOL"/*.deb; do dpkg-deb --field "$f" Architecture; done | sort -u)"
for arch in $archs; do
  dir="$DISTS/$COMPONENT/binary-$arch"
  install -d "$dir"
  # Paths inside Packages are relative to the repository root.
  (
    cd "$REPO"
    apt-ftparchive --arch "$arch" packages "pool/$COMPONENT/p/panora" > "dists/$SUITE/$COMPONENT/binary-$arch/Packages"
  )
  gzip -9nkf "$dir/Packages"
done

archs_line="$(echo "$archs" | tr '\n' ' ' | sed 's/ *$//')"
(
  cd "$DISTS"
  apt-ftparchive \
    -o "APT::FTPArchive::Release::Origin=Panora" \
    -o "APT::FTPArchive::Release::Label=Panora" \
    -o "APT::FTPArchive::Release::Suite=$SUITE" \
    -o "APT::FTPArchive::Release::Codename=$SUITE" \
    -o "APT::FTPArchive::Release::Architectures=$archs_line" \
    -o "APT::FTPArchive::Release::Components=$COMPONENT" \
    -o "APT::FTPArchive::Release::Description=Panora clipboard manager" \
    release . > Release.new
  mv Release.new Release
)

key="${APT_GPG_KEY_ID:-$(gpg --batch --list-secret-keys --with-colons 2>/dev/null | awk -F: '/^sec/ { print $5; exit }')}"
if [ -z "$key" ]; then
  echo "apt-repo.sh: no GPG secret key; Release left unsigned" >&2
  rm -f "$DISTS/Release.gpg" "$DISTS/InRelease"
  exit 0
fi

gpg_sign() {
  if [ -n "${APT_GPG_PASSPHRASE:-}" ]; then
    gpg --batch --yes --pinentry-mode loopback --passphrase "$APT_GPG_PASSPHRASE" --local-user "$key" "$@"
  else
    gpg --batch --yes --local-user "$key" "$@"
  fi
}
gpg_sign --armor --detach-sign --output "$DISTS/Release.gpg" "$DISTS/Release"
gpg_sign --armor --clearsign --output "$DISTS/InRelease" "$DISTS/Release"
gpg --batch --yes --export "$key" > "$REPO/panora.gpg"

cat > "$REPO/index.html" <<'HTML'
<!doctype html>
<meta charset="utf-8">
<title>Panora APT repository</title>
<pre>
curl -fsSL https://ygkali.github.io/panora/apt/panora.gpg | sudo tee /usr/share/keyrings/panora.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/panora.gpg] https://ygkali.github.io/panora/apt stable main" | sudo tee /etc/apt/sources.list.d/panora.list
sudo apt update && sudo apt install panora
</pre>
HTML

echo "apt-repo.sh: $REPO ready ($(echo "$archs_line") ; key $key)"
