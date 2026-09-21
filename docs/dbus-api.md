# The public D-Bus API

Panora's primary, internal protocol is the private Unix socket at
`$XDG_RUNTIME_DIR/panora.sock` (`panora_core::ipc`; `panora-cli` and the popup
both speak it). `panod` additionally owns `io.github.ygkali.Panora1` on the
**session bus**, for third-party integrations that would rather speak D-Bus
than open that socket themselves — a Waybar module, a shell script through
`gdbus`/`dbus-send`, a status bar widget.

This is a thin mirror, not a second implementation: every method call
becomes exactly the same request the Unix socket accepts, so there is
exactly one place the privacy gates, encryption and deduplication logic
live. If `panod` is not running, the service is not on the bus at all.

- **Bus name:** `io.github.ygkali.Panora1`
- **Object path:** `/io/github/ygkali/Panora1`
- **Interface:** `io.github.ygkali.Panora1`

## Methods

```
List(a{sv} filters) → aa{sv} entries
Recall(x id, b paste, s mime) → b pasted
Pin(x id, b pinned)
Delete(x id)
Clear() → u count
SetPrivate(b enabled)
Status() → a{sv}
```

### `List`

`filters` (all optional, same meaning as `panora-cli list`'s flags):

| Key | Type | |
|---|---|---|
| `search` | `s` | Search string; the same grammar as the CLI (`kind:`, `app:`, `pinned:`, `before:`/`after:`, `re:`, quoted phrases). |
| `kind` | `s` | One of `text`, `richtext`, `link`, `image`, `files`, `color`, `binary`. |
| `pinned_only` | `b` | Only pinned entries. |
| `limit` | `u` | Page size (0 or omitted = 50, capped at 500). |
| `offset` | `u` | Page offset. |

Each returned entry is a dict with `id` (`x`), `content_hash` (`s`),
`preview` (`s`), `kind` (`s`), `primary_mime` (`s`), `size_bytes` (`x`),
`source_app` (`s`, empty if unknown), `created_at` (`x`), `last_seen_at`
(`x`), `pinned` (`b`), `sensitive` (`b`), `selection` (`s`,
`"clipboard"`/`"primary"`).

While the second-layer lock (SEC-02) is engaged, `List` returns an empty
array — a `aa{sv}` reply has nowhere to carry "count only" the way the Unix
socket's `List` degrades to; call `Status` for the count instead.

### `Recall`

Puts an entry back on the clipboard. `mime` empty means every stored
format; a specific MIME type (e.g. `text/plain`) offers only that one.
Returns whether a paste keystroke was delivered (meaningless when `paste`
is `false`). Refused while the second-layer lock is engaged.

### `Pin` / `Delete` / `Clear` / `SetPrivate`

Same as the CLI commands of the same name. `Clear` deletes every unpinned
entry and returns how many were removed.

### `Status`

A dict with `backend` (`s`), `entries` (`x`), `private_mode` (`b`),
`sync_active` (`b`), `revision` (`t`), `version` (`s`), `protocol` (`u`,
the *Unix socket's* protocol version), `locked` (`b`, the OS session lock),
`app_locked` (`b`, the second-layer lock's state) and `lock_password_set`
(`b`).

## Signal: `Changed`

```
Changed(t revision)
```

Fired every time the history changes, carrying the new revision — the
D-Bus equivalent of the Unix socket's `Subscribe` event stream. A client
that wants to react to changes should watch this instead of polling
`Status`.

## Trying it from a shell

```sh
gdbus call --session \
  --dest io.github.ygkali.Panora1 \
  --object-path /io/github/ygkali/Panora1 \
  --method io.github.ygkali.Panora1.Status

gdbus call --session \
  --dest io.github.ygkali.Panora1 \
  --object-path /io/github/ygkali/Panora1 \
  --method io.github.ygkali.Panora1.List '{}'

gdbus monitor --session --dest io.github.ygkali.Panora1
```

## What is intentionally not here

Payload content (`Preview` on the Unix socket) is not exposed over D-Bus:
the session bus is a broadcast medium any other application on it can
introspect and monitor, which is a worse place to put clipboard content
through than a private, 0600 Unix socket only the owning user's own
processes can open. `Recall` still works — it puts the content on the
clipboard, which is exactly where every other application already expects
to read it from — but a method that hands back raw payload bytes over the
bus does not exist and is not planned. Export/import, the second-layer
lock's password management, key rotation and the panic wipe are Unix-socket
(`panora-cli`) only for the same reason: none of them belong on a bus other
processes can watch.
