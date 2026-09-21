# config.toml reference

`~/.config/panora/config.toml` (or `$XDG_CONFIG_HOME/panora/config.toml`).
Every key has a default, so the file only needs the ones you change, and a
missing file is the same as an empty one. The settings dialog writes this
file; `panora-cli reload` — which the dialog calls for you — makes the
daemon re-read it without restarting.

An invalid value is refused with the key named, and the daemon keeps the
configuration it already had rather than starting with something you did not
ask for.

## `[history]`

```toml
[history]
record_primary = false
max_entries = 1000
max_age_days = 30
max_mime_bytes = 10485760
persist_on_wayland = "auto"
index_full_text = true
max_total_bytes = 536870912
max_images = 200
```

| Key | Default | What it does |
|---|---|---|
| `record_primary` | `false` | Also record the PRIMARY selection — text you merely select with the mouse. Noisy by nature; off unless you want it. |
| `max_entries` | `1000` | Unpinned entries kept. The oldest go first. Pinned entries do not count. |
| `max_age_days` | `30` | Unpinned entries older than this are removed. `0` keeps them forever. |
| `max_mime_bytes` | `10485760` (10 MiB) | Most bytes read for one payload. A larger offer is skipped, not truncated. |
| `persist_on_wayland` | `"auto"` | Re-offer the last entry when the clipboard empties because the source application exited. `auto` turns it on for compositors that drop the selection (Sway, Hyprland and the other wlroots ones) and leaves Mutter and KWin alone; `always` and `never` override the detection. |
| `index_full_text` | `true` | Index up to 64 KiB of an entry's text, not just its 500-character preview, so search finds words anywhere in it. The index is in the same 0600 file. |
| `max_total_bytes` | `536870912` (512 MiB) | Payload bytes the history may hold. Over the limit, the oldest unpinned entries go first; pinned entries count towards it but are never evicted. `0` means no limit. |
| `max_images` | `200` | Image entries kept, oldest unpinned first. `0` means no limit. |

## `[privacy]`

```toml
[privacy]
start_private = false
excluded_apps = ["keepassxc", "bitwarden", "1password", "org.keepassxc", "com.bitwarden", "secrets"]
excluded_window_titles = []
min_text_length = 1
ignore_whitespace_only = true
ignore_patterns = []
capture_kinds = []
sensitive_policy = "mask"
sensitive_ttl_minutes = 10
lock_after_idle_minutes = 0
```

| Key | Default | What it does |
|---|---|---|
| `start_private` | `false` | Start with recording paused. |
| `excluded_apps` | the three password managers (plus their reverse-DNS app id variants) and `secrets` (the gnome-keyring prompt) | Source application names that are never recorded. Matched case-insensitively as a substring, so `keepass` covers `keepassxc`. The one list lives in `panora_core::privacy::DEFAULT_EXCLUDED_APPS`; this default is generated from it, not maintained separately (SEC-10). Needs a session that can name the source — see below. |
| `excluded_window_titles` | `[]` | Phrases; nothing is recorded while the focused window's title contains one, case-insensitively. The title itself is never stored. X11 and the GNOME extension report titles. |
| `min_text_length` | `1` | Text shorter than this many characters, after trimming, is not recorded. |
| `ignore_whitespace_only` | `true` | Skip text that is nothing but whitespace. |
| `ignore_patterns` | `[]` | Rust regular expressions; matching text is not recorded. At most 32 patterns, 512 bytes each. A pattern that does not compile is refused when you save it, not silently ignored. |
| `capture_kinds` | `[]` | Kinds to record: `text`, `richtext`, `link`, `image`, `files`, `color`, `binary`. Empty means all of them. |
| `sensitive_policy` | `"mask"` | What happens to text that looks like a key, a token, a card number or an IBAN. `mask` records it with a masked *preview* and no full-text index — the real content is still stored and still comes back on recall/preview/export, like any other entry; this is a display policy, not a way to make the secret itself inaccessible. `drop` never records it at all. `store` treats it like anything else, preview included. |
| `sensitive_ttl_minutes` | `10` | Flagged entries are removed after this long, under `mask` and `store` alike. `0` leaves them to the normal retention rules. Pinned entries stay either way. |
| `lock_after_idle_minutes` | `0` | Engage the second-layer lock (`panora-cli lock`, SEC-02) on its own after this many minutes of inactivity. `0` disables idle locking; a lock password has to be set first either way (`panora-cli lock set-password`), or this does nothing. |

`excluded_apps` is enforced on the MIME list **before** a payload is read,
along with the secret markers password managers set
(`x-kde-passwordManagerHint`, `org.nspasteboard.concealedtype`, the Qt
*clipboard viewer ignore* type) — those work on every session type. The application name does not: a plain Wayland
compositor without `wlr-foreign-toplevel-management` cannot say who owns
the clipboard, so the list has nothing to match. `panora-cli status` prints
`source_app=false` there and the settings dialog says so next to the list.

```toml
# some patterns worth having
ignore_patterns = [
  '^\d{16}$',            # a bare card number
  '^[A-Z]{2}\d{2}[A-Z0-9]{10,30}$',  # a bare IBAN
  '^-----BEGIN [A-Z ]*PRIVATE KEY-----',
]
```

## `[ui]`

```toml
[ui]
language = "system"
theme = "system"
instant_paste = false
close_on_focus_loss = true
position = "pointer"
layer_anchor = "top-right"
```

| Key | Default | What it does |
|---|---|---|
| `language` | `"system"` | `system`, `tr` or `en`. `system` follows `LC_ALL` / `LC_MESSAGES` / `LANG`. |
| `theme` | `"system"` | `system`, `light` or `dark`. |
| `instant_paste` | `false` | Send a paste keystroke to the focused window after picking an entry — `Ctrl+V`, or `Ctrl+Shift+V` when that window is a terminal. Needs a session that can synthesise keys (`paste=true` in `panora-cli status`). |
| `close_on_focus_loss` | `true` | Close the popup when focus moves to another window, the way the Win+V flyout does. |
| `position` | `"pointer"` | `pointer` or `center`. Applies on X11 and, through the Shell extension, on GNOME. Other Wayland compositors decide for themselves. |
| `layer_anchor` | `"top-right"` | Which corner the popup is anchored to on wlroots compositors when `gtk4-layer-shell` is available: `top-right`, `top-left`, `bottom-right`, `bottom-left`, `center`. |

## Where everything lives

| Path | What |
|---|---|
| `~/.config/panora/config.toml` | this file |
| `~/.local/share/panora/history.db` | the encrypted history, mode 0600 in a 0700 directory |
| `~/.local/share/panora/history.db.bak-vN` | the copy taken before a schema migration |
| `$XDG_RUNTIME_DIR/panora.sock` | the IPC socket, mode 0600, peer UID checked |
| the Secret Service | the master key, under the `application=panora` attribute |

The daemon writes nothing outside these. There is no system-wide
configuration file and no root-owned state.
