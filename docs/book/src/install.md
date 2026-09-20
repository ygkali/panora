# Install

Panora needs **GTK 4.12** and **libadwaita 1.5**, which means Debian 13,
Ubuntu 24.04, Zorin OS 18 or newer. Ubuntu 22.04, Zorin OS 17, Mint 21 and
Debian 12 are too old and the package refuses to install on them.

[How Panora is distributed](distribution.md) has the full list of tested
distributions, the support window, and how to verify a download.

## From the APT repository

The one to pick if you want `apt upgrade` to bring you new versions.

```sh
curl -fsSL https://ygkali.github.io/panora/apt/panora.gpg \
  | sudo tee /usr/share/keyrings/panora.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/panora.gpg] https://ygkali.github.io/panora/apt stable main" \
  | sudo tee /etc/apt/sources.list.d/panora.list
sudo apt update && sudo apt install panora
systemctl --user enable --now panod.service
```

## From the Debian package

Download `panora_<version>_<arch>.deb` from the
[releases page](https://github.com/ygkali/panora/releases) — amd64 and
arm64 — check it against `SHA256SUMS`, then:

```sh
sha256sum -c SHA256SUMS --ignore-missing
sudo apt install ./panora_*.deb
systemctl --user enable --now panod.service
```

## From source

```sh
sudo apt install -y build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
  binutils libglib2.0-bin adwaita-icon-theme librsvg2-common gnome-keyring
git clone https://github.com/ygkali/panora.git && cd panora
./packaging/build-deb.sh
sudo apt install ./dist/panora_*.deb
```

Rust 1.92 or newer. `./install.sh` does all of the above plus the service
and the extension, in English or Turkish depending on your locale.

## After installing

On GNOME, log out and back in once so the Shell notices the extension, then:

```sh
gnome-extensions enable panora@ygkali.github.io
```

The extension binds **Super+V** (taking it from the notification list) and,
on GNOME versions without a data-control protocol, is what lets the daemon
see the clipboard at all. The popup shows a banner with an **Enable**
button when it is installed but not running.

On every other desktop, bind a key to `panora-cli toggle` yourself;
[What works where](desktops.md) says where each desktop hides that setting.

Then check the whole thing:

```sh
panora-doctor
```

It reports the session type, the backend in use, the service state, whether
the keyring is unlocked, whether the extension is enabled, and the
permissions on the data directory. Anything it complains about has a fix
printed next to it.

## Removing it

```sh
sudo apt remove panora        # or ./uninstall.sh
```

Your history stays in `~/.local/share/panora` unless you pass `--purge-data`
to `uninstall.sh`. The master key stays in the keyring; removing it makes
an old history file unreadable, which is the point.
