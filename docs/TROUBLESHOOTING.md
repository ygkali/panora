# Troubleshooting

Start with `panora-doctor`: it checks the session, the daemon, the socket, the
backend, the GNOME extension, the D-Bus names, the keyring, the Super+V
binding and the data directory, and prints a command for everything that is
not OK. `panora-doctor --json` is what to paste into a bug report; it contains
no clipboard content.

```sh
systemctl --user status panod.service
journalctl --user -u panod.service -n 100 --no-pager
panora-cli status
```

## `panora-cli: daemon unavailable`

panod is not running or its socket is elsewhere. Read the error in
`systemctl --user status panod.service`, then
`systemctl --user enable --now panod.service`.

## The keyring is locked

panod takes its master key from the Secret Service (gnome-keyring, KWallet).
When the login keyring is locked an unlock prompt appears; if nobody answers
it, panod gives up after 60 s with `Secret Service did not answer within 60s`.
Unlock the keyring, then:

```sh
systemctl --user reset-failed panod.service
systemctl --user restart panod.service
```

The unit stops retrying after three failures (`StartLimitBurst=3`) so a
locked keyring does not raise prompt after prompt.

## `the history was encrypted with a different master key`

The keyring was reset or replaced, so the key that encrypted
`~/.local/share/panora/history.db` is gone. Restore the keyring from a backup
if you have one; otherwise move the data directory away and let panod start a
new, empty history:

```sh
mv ~/.local/share/panora ~/.local/share/panora.old
systemctl --user restart panod.service
```

## `Wayland compositor has no data-control protocol`

Normal on GNOME 47 and older: panod switches to the GNOME bridge backend and
needs the Shell extension. Without it nothing is captured on those releases.

```sh
gnome-extensions enable panora@ygkali.github.io
gnome-extensions info panora@ygkali.github.io
```

The Shell only discovers a newly installed system extension after it
restarts, so log out and back in once after installing the package.

## The service never starts, `status=226/NAMESPACE`

The unit opens `~/.local/share/panora` with `ReadWritePaths` and creates it in
`ExecStartPre`. If the directory was removed or its permissions changed by
hand:

```sh
install -d -m 0700 ~/.local/share/panora
systemctl --user restart panod.service
```

## Super+V does nothing

The extension is installed by the package but only enabled after the Shell
restarts. Log out and back in, then enable it (above). GNOME binds Super+V to
the notification list by default; the extension removes that binding while
it is enabled and restores it when disabled. Without the extension panod
still records everything on GNOME 48+ and other desktops; open the popup
with `panora` or bind `panora-cli toggle` to a key in your desktop's shortcut
settings.

## Icons show as broken boxes

`librsvg2-common` is missing (Adwaita ships symbolic icons as SVG only):

```sh
sudo apt install -y librsvg2-common adwaita-icon-theme
```

## Instant paste does not work

X11 uses XTEST, GNOME the extension; other Wayland desktops need `wtype`
(wlroots) or `ydotool` with its uinput daemon. When the keystroke cannot be
delivered the entry is still put on the clipboard and a notification says so.
Terminals expect Ctrl+Shift+V rather than Ctrl+V; see `docs/ROADMAP.md`
INT-09.

## The exclusion list does not apply on Wayland

The `ext`/`wlr-data-control` protocols do not tell which application owns the
clipboard, so the `excluded_apps` list cannot fire on KDE, Sway, Hyprland and
GNOME 48+ native capture; `panora-cli status` reports `source_app=false`. The
password manager MIME gate applies everywhere, and the GNOME extension
supplies the application name on GNOME.

## X forwarding over SSH

`panod.service` blocks TCP with `RestrictAddressFamilies=AF_UNIX`, which a
`DISPLAY=localhost:10` needs. Remove that line from
`/usr/lib/systemd/user/panod.service` (or a drop-in) and run
`systemctl --user daemon-reload`.

## Where things live

| Path | Content |
|---|---|
| `~/.config/panora/config.toml` | Settings (0600) |
| `~/.local/share/panora/history.db` | Entry metadata, encrypted previews, FTS index (0600) |
| `~/.local/share/panora/blobs/` | Encrypted payloads, content-addressed |
| `~/.local/share/panora/history.db.bak-vN` | Backup taken before a schema migration |
| `$XDG_RUNTIME_DIR/panora.sock` | IPC socket (0600) |
