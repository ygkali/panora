# Syncing between your devices (experimental)

`panora-sync` keeps the clipboard history of your own computers in step
over the local network. It is a separate package and a separate service:
without it installed, Panora contains no network code at all, and with it
installed it stays off until you turn it on.

**Status:** experimental. The protocol has been reviewed internally but not
yet by an independent security review (ROADMAP SYNC-06). Use it on networks
you trust, such as your home network.

## What it does, and what it does not

- Devices find each other with mDNS on the local network, or at addresses
  you list, and talk QUIC (TLS 1.3). Only loopback, private (`10/8`,
  `172.16/12`, `192.168/16`), link-local and IPv6 unique-local addresses
  are ever contacted or accepted, whatever mDNS or the configuration says.
  There is no relay and no server: devices on different networks do not
  sync (yet).
- Every entry is additionally sealed with the group's key before it leaves
  the device. When you remove a device, the group key is replaced, so the
  removed device cannot read what is shared afterwards.
- Sensitive entries (passwords, tokens detected by `sensitive_policy`) never
  leave the device. Entries arriving from another device go through the same
  privacy rules as a local copy: excluded applications, private mode and
  content filters apply on the receiving side too.
- Deletions and pins travel. A deletion is remembered for
  `sync.tombstone_days` (30 by default); a device offline for longer than
  that may bring a deleted entry back.

## Setting it up

Install the package and start the service on each device:

```bash
sudo apt install ./panora-sync_*.deb
systemctl --user enable --now panora-sync
```

### In Panora

Everything below is also in the popup: menu › **Devices** (or
Preferences › Devices). The page shows whether the service runs, with a
switch to start it, this device's name and fingerprint, and the devices
of its group, connected or not, each with a button to remove it.

- On a device already in the group (or on the first one), **Pair with a
  code** waits for the other device and shows six digits; **Invitation
  link** shows a link to copy and a QR code. The copied link is marked
  as a secret, so Panora keeps it out of the history.
- On the new device, **Join with a code** finds the waiting device on the
  network (or takes its address) and shows the same six digits; **Join
  with an invitation link** takes the pasted link.
- Compare the codes and choose **Same, pair** on both screens only if they
  match. Going back, or closing the preferences, cancels what is in
  progress.

The hint on the copied link only helps on the device that copied it. On
the device you paste it into, it usually arrives through a messenger or a
note, as an ordinary copy that Panora records like any other. It is
single-use and expires after ten minutes, but you can delete that entry
once the device has joined.

The same, from a terminal (it answers in the language set in the
preferences, like `panora-cli`; `--help` and the manual page are in
English):

On the first device, invite the second one:

```bash
panora-sync invite
```

It prints a link and a QR code. On the second device:

```bash
panora-sync join 'panora-pair:1?id=...'
```

The link carries a one-time secret and the first device's key: it works
once, for ten minutes, and anyone who sees it in that time can join. Treat
it like a password.

Without copying a link across, compare a code instead. On a device already
in the group:

```bash
panora-sync pair
```

and on the new one:

```bash
panora-sync join --code
```

Both screens show the same six digits; answer **y** on both only if they
match. If they differ, someone is in between: answer **n**. A device
waiting with `pair` accepts at most three attempts in five minutes.

A third device can join through any device already in the group; the others
learn about it on their own.

## Day to day

```bash
panora-sync status          # this device, the group, who is connected
panora-sync remove laptop   # by name, or the start of its fingerprint
panora-sync leave           # on this device; remove it from another one too
```

A lost or stolen device: remove it from any other device. Its copy of the
history stays on it, but it gets nothing new, and cannot rejoin by itself.

## Configuration

In `~/.config/panora/config.toml`:

```toml
[sync]
# Turned on by panora-sync when this device joins a group.
enabled = true
# Days a deletion is remembered for devices that were offline.
tombstone_days = 30
# UDP port; 0 picks a free one (mDNS announces it).
port = 47100
# For networks that block mDNS: other devices as ip:port. No host names.
peers = ["192.168.1.20:47100"]
# Find the other devices with mDNS.
discovery = true
# Share only pinned entries, or only text-like ones.
pinned_only = false
text_only = false
```

Restart the service after editing: `systemctl --user restart panora-sync`.

## Troubleshooting

- `panora-sync status` shows **not connected**: check that both devices are
  on the same network, and that the firewall allows UDP on the sync port
  (and UDP 5353 for mDNS). On networks that block mDNS, list the other
  device in `sync.peers`.
- `journalctl --user -u panora-sync` shows what the service is doing;
  `Environment=RUST_LOG=panora_sync=debug` in an override
  (`systemctl --user edit panora-sync`) shows more.
- The state (this device's key, the device list, the group key) is in
  `~/.local/share/panora/sync/state.bin`, sealed with a key kept in your
  keyring.

The design and its limits are recorded in `docs/adr/0004-sync-transport.md`
and `docs/adr/0005-pairing-and-group-key.md`.
