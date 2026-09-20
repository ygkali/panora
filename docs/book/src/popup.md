# The popup

Open it with **Super+V** on GNOME, with the launcher entry, by running
`panora`, or with `panora-cli toggle` from any key binding. Opening it again
closes it, and it closes by itself when you move to another window — the
Win+V habit, which `ui.close_on_focus_loss` turns off if you would rather it
stayed.

## Keys

| Key | Action |
|---|---|
| `Ctrl+F` | Focus the search box. Typing anywhere in the panel starts a search too |
| `↑` `↓` `←` `→`, `Home` `End` `PgUp` `PgDn` | Move between entries |
| `Enter` | Put the entry on the clipboard and close; with instant paste on, also paste it |
| `Shift+Enter` | Put **only the plain text** on the clipboard, dropping the HTML of a rich-text entry |
| `Ctrl+1` … `Ctrl+9` | Pick the Nth entry; the first nine rows show their number |
| `Space` | Details: the full text, the full-size image, every format the entry carries |
| `Ctrl+D` | Pin or unpin |
| `Delete` | Delete; the toast offers **Undo** for thirty seconds |
| `Ctrl+Shift+P` | Private mode on or off |
| `Ctrl+,` | Settings |
| `Esc` | Clear the search, or close when the search is already empty |

## The rows

Each row shows what kind of entry it is, a preview, which application the
copy came from (where the session can tell), and how long ago it was last
used. A pin keeps an entry through **clear history** and through retention;
a lock icon marks an entry the privacy engine flagged as sensitive, which
is masked in the list and removed again after
`privacy.sensitive_ttl_minutes`.

The filter chips above the list — all, pinned, text, links, images, files,
rich text, colours — are the same thing as a `kind:` search, one click
away.

## Details

`Space` opens the details view of the selected entry. What is there depends
on the kind:

- **Text and rich text**: the whole text, the list of formats, *copy* and
  *copy as plain text*.
- **Links**: *open* in the default browser, and a QR code for sending the
  link to a phone.
- **Colours**: the swatch plus hex, RGB and HSL, each one click to copy.
- **Images**: the image at full size and *save as*.
- **Files**: the paths, and *open the containing folder*.

## Private mode

The switch in the header bar, `Ctrl+Shift+P`, or `panora-cli private on`.
While it is on, nothing new is recorded — the daemon still serves what is
already there, so you can recall an old entry without leaving a trace of
what you copy in between. It survives a configuration reload and is
reported by `panora-cli status`.

Recording also pauses by itself while the screen is locked.

## The shortcut

On GNOME the shortcut belongs to the extension, not to the popup: open the
Extensions app or run `gnome-extensions prefs panora@ygkali.github.io`,
click the shortcut row and press the combination you want. `Backspace`
disables it, `Escape` cancels, and the reset button puts `Super+V` back.
The same page has the *open next to the pointer* switch.

Everywhere else, bind a key to `panora-cli toggle` in your desktop's own
keyboard settings -- [What works where](desktops.md) says where each one
hides it.

## Settings

`Ctrl+,` opens the settings dialog, which writes
`~/.config/panora/config.toml` and tells the daemon to re-read it — nothing
needs restarting. Every key it writes, and the several it does not expose,
are in the [config.toml reference](config.md).
