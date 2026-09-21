# Command line

`panora-cli` talks to the daemon over the socket in `$XDG_RUNTIME_DIR`. It
is the scripting interface and the one to reach for when the popup will not
open.

```sh
panora-cli list [QUERY] [--kind image] [--pinned] [--limit 20] [--offset 20] [--format TEMPLATE]
panora-cli search <TEXT> [--format TEMPLATE]
panora-cli copy <ID> [--paste] [--mime text/plain] [--primary]
panora-cli preview <ID> [--mime image/png] [--out photo.png]
panora-cli pin <ID> | unpin <ID> | delete <ID> | restore <ID>
panora-cli clear
panora-cli private on|off
panora-cli status | stats | toggle | reload
panora-cli config get [KEY] | set <KEY> <VALUE> | validate | edit
panora-cli store [FILE] [--mime TYPE] [--app NAME] [--no-copy]
panora-cli pick [--format '{id}\t{kind}\t{preview}']
panora-cli watch
panora-cli rotate-key
panora-cli lock [set-password|change-password|remove-password] | unlock
panora-cli wipe --yes
panora-cli export --out FILE | import <FILE>
panora-cli import-legacy copyq|gpaste|clipboard-indicator
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

`panora-cli copy <ID> --primary` puts the entry on PRIMARY instead of
CLIPBOARD, so a middle-click pastes it; `--paste` is ignored with it, since
there is no keyboard shortcut for a PRIMARY paste. `privacy.
clear_clipboard_after_seconds` (`config.toml`, 0 = disabled) clears whichever
selection a recall was put on that many seconds later — the way a password
manager times out what it copied — but only if nothing else was copied in
the meantime.

`panora-cli stats` prints counts by kind, how many entries are pinned or
flagged sensitive, total payload bytes, and the oldest/newest entry's time.

## Watching the daemon

```sh
panora-cli status
```

```
backend=wayland entries=812 private=false locked=false app_locked=false
lock_password_set=false version=1.6.0 protocol=3 revision=57 primary=true
persist=true paste=true source_app=true needs_bridge=false
```

`backend` is the clipboard protocol in use, `revision` rises on every
change, and the capability flags say which features this session can offer
— see [What works where](desktops.md). Any health findings are printed
after it as `health=<code> <message>`; those are the same ones the popup
shows as a banner. `locked` is the OS session lock (screensaver); `app_
locked`/`lock_password_set` are the second-layer lock, below.

`panora-cli reload` re-reads `config.toml` without restarting, which is what
the settings dialog does when you press *save*.

### Reading and changing `config.toml` from a script

```sh
panora-cli config get                              # the whole file
panora-cli config get history.max_entries           # one key
panora-cli config set privacy.sensitive_ttl_minutes 30
panora-cli config set privacy.excluded_apps "keepassxc,bitwarden,1password"
panora-cli config validate
panora-cli config edit                               # opens $VISUAL/$EDITOR
```

Keys are the dotted path you would write in `config.toml` — see
[config.toml reference](config.md) for every one of them. `set` parses the
new value to match the existing key's type: `true`/`false`/`1`/`0`/`yes`/`no`
for a boolean, a plain integer, a comma-separated list for an array key
(`excluded_apps`, `ignore_patterns`, `capture_kinds`), anything else as
text — and refuses to write the file at all if the result would not pass
`config validate`. `set` and a successfully re-read `edit` both tell a
running daemon to pick up the change immediately (the same as
`panora-cli reload`), best-effort — nothing breaks if the daemon happens not
to be running right then, since it reads the file fresh on its own next
start anyway.

Instead of polling `status`, `panora-cli watch` blocks and prints a line
every time `revision` changes (the first line is always the current one, so
starting a watcher never misses a change that just happened):

```sh
panora-cli watch | while read -r _; do panora-cli list --limit 5; done
```

## Locking the history while the daemon keeps running

The master key always loads from the OS keyring unattended — `panod` has to
survive a reboot with no one there to unlock anything — but a lock password
gates a *running* daemon on top of that (SEC-02). Passwords are always read
from standard input, never a command-line argument:

```sh
printf '%s' 'a lock password' | panora-cli lock set-password
panora-cli lock            # engage now
printf '%s' 'a lock password' | panora-cli unlock
```

While engaged, `list`/`search` report a count only and `preview`/`copy` are
refused; `privacy.lock_after_idle_minutes` engages it on its own after that
much inactivity. `lock change-password`/`lock remove-password` work the
same way, reading one or two lines from standard input as documented in
`panora-cli lock --help`.

## Backup and restore

```sh
printf '%s' 'a backup passphrase' | panora-cli export --out backup.panora
printf '%s' 'a backup passphrase' | panora-cli import backup.panora
```

The archive is encrypted with a key derived from the passphrase (Argon2id);
there is no way to recover a forgotten one. `import` bypasses the privacy
gate — the archive is your own previously-exported data, not a live capture
to screen — and deduplicates by content hash, so importing the same archive
twice or into a history that already has some of it never duplicates
anything.

`panora-cli import-legacy copyq|gpaste|clipboard-indicator` reads another
clipboard manager's history instead, through the normal privacy-gated path
(so excluded apps and ignore patterns still apply). This one is best-effort
against each tool's documented output format, since none of the three are
part of this project's own test environment.

## If something goes wrong

`panora-cli rotate-key` generates a new master key and reseals the whole
history under it; safe to interrupt (Ctrl+C, a crash, a daemon restart) and
re-run, since it resumes the same rotation instead of starting a new one.

`panora-cli wipe --yes` hard-deletes the whole history — pinned entries
included, no undo — and retires the encryption key for a fresh one, so
what was just deleted is unrecoverable from the file system too. It works
regardless of the second-layer lock's state on purpose: a panic action has
to work under duress, not only after unlocking first.
