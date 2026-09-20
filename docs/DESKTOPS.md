# Panora on each desktop

What works where, how to bind the shortcut, and what to install. The
daemon, the popup and the CLI are the same everywhere; the differences are
in how the clipboard is watched, whether the copying application is known,
and how the popup is opened and placed.

| | GNOME (Wayland) | GNOME (Xorg) | KDE Plasma (Wayland) | Sway, Hyprland, other wlroots | Xfce, MATE, Cinnamon, LXQt |
|---|---|---|---|---|---|
| Capture | Shell extension on GNOME ≤ 47; `ext-data-control` on GNOME 48+ | X11 backend | `ext-data-control` / `wlr-data-control` | `wlr-data-control` | X11 backend |
| Source application (excluded apps, `app:` search) | yes (extension) | yes (`WM_CLASS`) | yes (`wlr-foreign-toplevel`) | yes (`wlr-foreign-toplevel`) | yes (`WM_CLASS`) |
| Window titles (`excluded_window_titles`) | yes (extension) | yes (`_NET_WM_NAME`) | yes | yes | yes |
| Instant paste | extension (`Ctrl+V`, `Ctrl+Shift+V` in terminals) | XTEST | `wtype` or `ydotool` | `wtype` or `ydotool` | XTEST |
| Clipboard after the source exits | kept by Mutter | re-offered by panod | kept by KWin | re-offered by panod | re-offered by panod |
| Popup placement | next to the pointer (extension) | next to the pointer | compositor decides | corner overlay with the `layer-shell` build, otherwise compositor decides | next to the pointer |
| Shortcut | extension binds `Super+V` | extension or the desktop's own settings | System Settings → Shortcuts | compositor config | the desktop's keyboard settings |

`panora-doctor` reports which of these apply to the running session.

## GNOME

The `.deb` installs the Shell extension; `install.sh` enables it and takes
`Super+V` over from the notification list. On GNOME ≤ 47 Wayland every copy
reaches the daemon through the extension, so it must be enabled (the popup
shows a banner with an **Enable** button when it is not running). On GNOME
48 and later the daemon watches the clipboard itself; the extension still
provides the shortcut, the source application and the popup placement.

The extension has its own preferences -- the Extensions app, or
`gnome-extensions prefs panora@ygkali.github.io`: the shortcut, which you
can record by pressing it rather than by editing dconf, and whether the
popup is moved under the pointer once it appears. Both are dconf keys, so
`gsettings set org.gnome.shell.extensions.panora move-to-pointer false`
still works. The history, the privacy rules and the appearance belong to
the daemon and are set in the popup itself.

## KDE Plasma

Plasma 6 offers `ext-data-control-v1`, so panod captures natively and KWin
names the focused window through `wlr-foreign-toplevel-management`. Klipper
keeps running alongside; to avoid two histories, disable Klipper's history in
its settings or leave it for the tray and use Panora for search and pins.
The master key is stored through the Secret Service API, which KWallet
provides (`kwalletmanager` is in `Recommends`).

Shortcut: System Settings → Shortcuts → Add Command → `panora-cli toggle`,
assign `Meta+V`. Instant paste needs `wtype` (`sudo apt install wtype`).

## Sway, Hyprland and other wlroots compositors

Capture uses `wlr-data-control`; the activated toplevel names the source
application. Bind the toggle in the compositor:

```
# sway
bindsym $mod+v exec panora-cli toggle

# hyprland
bind = SUPER, V, exec, panora-cli toggle
```

The popup opens as a layer-shell overlay anchored to a screen corner
(`ui.layer_anchor`, default `top-right`) when it is built with the
`layer-shell` feature, which needs `libgtk4-layer-shell`:

```sh
cargo install --git https://github.com/ygkali/panora panora-gui --features layer-shell
```

Ubuntu 24.04 does not package the library, so the `.deb` is built without
the feature; on those systems the compositor places the popup like any
window (Sway: `for_window [app_id="io.github.ygkali.Panora"] floating enable`).

For a picker without the GTK popup, `panora-cli pick` prints one line per
entry for fuzzel, wofi, rofi or dmenu:

```sh
panora-cli pick | fuzzel --dmenu | cut -f1 | xargs -r panora-cli copy --paste
```

## Xfce, MATE, Cinnamon, LXQt

These run on Xorg, where panod watches the clipboard with XFixes and names
the source from `WM_CLASS`; instant paste goes through XTEST. Bind the
shortcut in the desktop's keyboard settings to `panora-cli toggle`
(Xfce: Settings → Keyboard → Application Shortcuts; MATE: Keyboard
Shortcuts → Custom; Cinnamon: Keyboard → Shortcuts → Custom; LXQt:
Shortcut Keys). The popup opens next to the pointer.

## Everything else

Any Wayland compositor with `ext-data-control-v1` or `wlr-data-control-v1`
works for capture; without either, GNOME's extension route is the only one.
Any X11 session works. `panora-doctor` says which backend was picked and
what the session cannot do.
