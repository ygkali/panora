# Panora

Panora is a clipboard history for the GNOME desktop. Press **Super+V**, the
last few hundred things you copied are there, pick one and it is on the
clipboard again.

What makes it different from the other clipboard managers is where the
history lives. Everything you copy ends up in a SQLite database encrypted
with a key held by the Secret Service (your keyring), the daemon runs as a
systemd **user** service with a tight sandbox, and nothing ever leaves the
machine — there is no network code in the daemon or in the GNOME Shell
extension, and CI fails the build if any appears.

```
Super+V  ->  panora-gui  ->  panod  ->  history.db (ChaCha20-Poly1305)
                               |            ^
                               |            +-- master key in the Secret Service
                               +-- X11 / Wayland / GNOME Shell clipboard
```

## Where to start

| If you want to | Read |
|---|---|
| install it | [Install](install.md) |
| know the shortcuts | [The popup](popup.md) |
| find one old thing among thousands | [Search syntax](search.md) |
| script it, or wire it into rofi or fuzzel | [Command line](cli.md) |
| change how much it keeps, or what it ignores | [config.toml reference](config.md) |
| know what it does with a password you copied | [Privacy and security](privacy.md) |
| use it on KDE, Sway or Xfce | [What works where](desktops.md) |
| fix something that is not working | [Troubleshooting](troubleshooting.md) |

## In one paragraph

The daemon (`panod`) watches the clipboard through whichever protocol the
session offers: the X11 selection, `ext-data-control` or `wlr-data-control`
on Wayland, or the GNOME Shell extension on GNOME versions that have
neither. Every offer is judged before a byte of it is read — the privacy
engine looks at the MIME list, the source application and the window title
first — and what survives is encrypted and stored. The popup
(`panora-gui`) and the command line (`panora-cli`) talk to the daemon over a
Unix socket in your runtime directory; the GNOME extension talks to it over
the session bus. Nothing runs as root and nothing writes outside your home
directory.

## Reporting something

`panora-doctor` first: it checks the session, the service, the keyring, the
extension and the permissions, and `panora-doctor --report` writes the file
to attach. Issues and security reports go to
[the repository](https://github.com/ygkali/panora); `SECURITY.md` has the
disclosure process.
