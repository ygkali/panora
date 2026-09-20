# Command line

`panora-cli` talks to the daemon over the socket in `$XDG_RUNTIME_DIR`. It
is the scripting interface and the one to reach for when the popup will not
open.

```sh
panora-cli list [QUERY] [--kind image] [--pinned] [--limit 20] [--offset 20] [--format TEMPLATE]
panora-cli search <TEXT> [--format TEMPLATE]
panora-cli copy <ID> [--paste] [--mime text/plain]
panora-cli preview <ID> [--mime image/png] [--out photo.png]
panora-cli pin <ID> | unpin <ID> | delete <ID> | restore <ID>
panora-cli clear
panora-cli private on|off
panora-cli status | toggle | reload
panora-cli store [FILE] [--mime TYPE] [--app NAME] [--no-copy]
panora-cli pick [--format '{id}\t{kind}\t{preview}']
panora-cli completions bash|zsh|fish
panora-cli man <DIR>
```

`--json` works on every command that prints something; `man panora-cli` has
the full text.

## Exit status

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | the daemon returned an error |
| 2 | wrong usage |
| 3 | the daemon is not running |
| 4 | no entry with that id |

Scripts can tell "there is no such entry" from "the daemon is down" without
parsing anything.

## Line templates

`pick` and the `--format` option on `list` and `search` print one line per
entry from a template. The placeholders are `{id}`, `{kind}`, `{preview}`,
`{app}`, `{age}`, `{size}`, `{pinned}` and `{sensitive}`; `\t` and `\n` are
escapes. The default is `{id}\t{kind}\t{preview}`.

That is all a launcher integration needs:

```sh
# fuzzel
panora-cli pick | fuzzel --dmenu | cut -f1 | xargs panora-cli copy --paste

# rofi
panora-cli pick --format '{id}\t{preview}' \
  | rofi -dmenu -i -p clipboard | cut -f1 | xargs panora-cli copy --paste

# dmenu, without the ids on screen
panora-cli pick --format '{preview}\t{id}' \
  | dmenu -l 15 | awk -F'\t' '{print $NF}' | xargs panora-cli copy
```

## Recording something that was never copied

```sh
git log -1 --format=%H | panora-cli store --app git
panora-cli store notes.txt --no-copy
```

`store` goes through the same privacy gate as a real copy: an excluded
application name, an ignore pattern or private mode all still apply.
Without `--no-copy` the entry also lands on the clipboard.

## Reading one back out

```sh
panora-cli preview 42                               # every format, with headers
panora-cli preview 42 --mime text/plain\;charset=utf-8   # just the payload
panora-cli preview 42 --mime image/png --out shot.png
```

With `--mime` the payload is written raw and nothing else, which is what
you want in a pipe.

## Watching the daemon

```sh
panora-cli status
```

```
backend=wayland entries=812 private=false locked=false version=1.3.0
protocol=2 revision=57 primary=true persist=true paste=true
source_app=true needs_bridge=false
```

`backend` is the clipboard protocol in use, `revision` rises on every
change (useful for polling), and the capability flags say which features
this session can offer — see [What works where](desktops.md). Any health
findings are printed after it as `health=<code> <message>`; those are the
same ones the popup shows as a banner.

`panora-cli reload` re-reads `config.toml` without restarting, which is what
the settings dialog does when you press *save*.
