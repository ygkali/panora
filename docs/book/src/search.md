# Search syntax

The search box in the popup and `panora-cli search` share one grammar.

- **Bare words match as prefixes.** `mer` finds *merhaba*. Several words all
  have to match, in any order.
- **`"quoted phrases"` match exactly**, in that order.
- **Operators narrow the result** and can be combined with words and with
  each other.

Matches are shown in bold in the popup.

## Operators

| Operator | Meaning |
|---|---|
| `kind:image` | one of `text`, `richtext`, `link`, `image`, `files`, `color` |
| `app:firefox` | the source application contains the word |
| `pinned:yes`, `pinned:no` | pinned or unpinned entries only |
| `after:7d`, `after:2026-09-01` | used since then |
| `before:12h`, `before:2026-09-01` | not used since then |
| `re:PATTERN` | the rest of the line is a case-insensitive regular expression |

Ages are `30m`, `12h`, `7d`, `2w`, or a `YYYY-MM-DD` day. `after:` and
`before:` both work on when the entry was **last used**, not when it was
first copied, so recalling an old entry moves it forward.

`app:` needs a session that can name the copying application: X11, GNOME
through the extension, and wlroots or KWin compositors through
`wlr-foreign-toplevel-management`. `panora-cli status` prints
`source_app=true` where it works. On a plain Wayland compositor without
that protocol the operator matches nothing, because nothing was recorded to
match against.

`re:` takes the whole rest of the line, so put it last. It is matched
against the preview and, when `history.index_full_text` is on, against the
indexed text of the entry.

## Examples

```
invoice kind:files after:7d          files with "invoice", used in the last week
app:firefox kind:link                links copied out of Firefox
"ssh -i" before:2w                   that command, from more than two weeks ago
pinned:yes kind:color                the palette you pinned
re:^https?://.*\.pdf$                entries that are a PDF URL and nothing else
```

## What is searched

The preview — the first 500 characters — always. The rest of the text only
when `history.index_full_text` is on, which it is by default; turning it off
makes the index smaller and the search shallower. Image and file entries are
searched through their file names and the application name.

The index is full-text (SQLite FTS5) and is written when an entry is
recorded, so the setting applies from the moment you change it: entries
captured while it was off keep only their preview indexed, and entries
captured while it was on keep their full text even after you turn it off.
