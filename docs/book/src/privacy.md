# Privacy and security

A clipboard manager sees everything you copy, including the things you
would never type into a text file. This page says what Panora does with
them, and — more usefully — what it does not defend against.

## The gate comes before the read

Nothing reads a clipboard payload until the privacy engine has judged the
offer. What it looks at is the list of formats, the source application and
the focused window's title — never the content, which has not been fetched
yet.

An offer is refused when:

- it carries a **secret marker** from a password manager:
  `x-kde-passwordManagerHint`, `org.nspasteboard.concealedtype`, the Qt
  *clipboard viewer ignore* type and their variants, matched
  case-insensitively and with MIME parameters ignored;
- the **source application** matches `privacy.excluded_apps` (KeePassXC,
  Bitwarden, 1Password and GNOME Secrets by default);
- the **focused window's title** contains one of
  `privacy.excluded_window_titles`;
- the kind is not in `privacy.capture_kinds`;
- **private mode** is on, or the screen is locked.

The markers work on every session type. The application name and the window
title need a session that reports them — X11, GNOME through the extension,
and wlroots or KWin compositors through
`wlr-foreign-toplevel-management`. Where they cannot be known,
`panora-cli status` says `source_app=false`, the settings dialog says so
next to the list, and the exclusion list simply has nothing to match. That
is a limitation of the protocol, not something Panora can work around, and
it is why the marker-based gate matters more than the name-based one.

## Text that looks like a secret

Content that gets past the gate is still examined for the shapes of API
keys, tokens, card numbers and IBANs. What happens then is
`privacy.sensitive_policy`:

- `mask` (the default) records it with a masked preview and keeps it out of
  the full-text index;
- `drop` does not record it at all;
- `store` treats it like anything else.

Under `mask` and `store` the entry is removed after
`privacy.sensitive_ttl_minutes` — ten minutes by default — so a token you
pasted once does not sit in the history for a month. Pinning one keeps it.

## At rest

| | |
|---|---|
| Cipher | XChaCha20-Poly1305, versioned envelope, associated data bound to the entry |
| Master key | in the Secret Service, fetched over an encrypted D-Bus session (`dh-ietf1024-sha256-aes128-cbc-pkcs7`) |
| No keyring | the daemon refuses to start rather than fall back to a key on disk |
| Wrong key | a database that does not match is refused with a clear message, never shown as garbage |
| Permissions | data directory 0700, files 0600, socket 0600 |
| IPC | Unix socket with peer UID checks, frame and request size limits |
| GNOME bridge | pushes accepted only from `org.gnome.Shell` |
| Process | `PR_SET_DUMPABLE=0`, `LimitCORE=0`, empty capability set, `ProtectProc=invisible`, `PrivateDevices`, `UMask=0077`, a `@system-service` syscall filter |
| Logs | a test asserts that no clipboard content ever reaches the daemon log |

Deleting an entry removes its payloads from disk unless another entry
shares them. A deletion keeps a tombstone for thirty seconds so **undo**
can bring it back; retention evictions and *clear* are final. Retention
runs on every store and once an hour.

## No network, and it is checked

The daemon and the GNOME Shell extension open no sockets beyond the session
bus and the local IPC socket. There is no telemetry, no update check and no
crash reporting. CI fails the build when a network primitive appears in
either — `scripts/security-check.sh` for the daemon, a grep for `fetch`,
`XMLHttpRequest`, `WebSocket` and URLs in the extension.

## What is outside the threat model

Panora protects the history file and what reaches it. It does not protect
you from:

- **the kernel and swap.** Clipboard content is in the daemon's memory while
  it is being handled; a swap file without encryption can hold it.
- **another process running as you.** The Unix permission model is the
  boundary. Anything running under your UID can read the socket, talk to
  the daemon and ask the keyring for the master key — that includes any
  program you install.
- **a malicious GNOME Shell extension.** The Shell can read the clipboard
  directly; Panora's extension is one of several.
- **an already compromised session.** If someone can log a keystroke, a
  clipboard manager is not the weak point.
- **forensic recovery.** SQLite can leave deleted rows in free pages until
  a `VACUUM`.

This is not an independent audit. `SECURITY.md` has the disclosure process,
`docs/adr/` the reasoning behind the design, and
`docs/security-checklist.md` each control with its evidence. RustSec
`cargo-audit` and `cargo-deny` run on every CI build.
