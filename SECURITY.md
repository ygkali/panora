# Security policy

Panora handles the most sensitive data on a desktop: whatever you copy. Reports
about anything that weakens that protection are welcome and taken seriously.

## Supported versions

| Version | Supported |
|---|---|
| 1.3.x (current) | Yes |
| 1.2.x and older | No, upgrade |

Only the latest minor release receives fixes. Security fixes ship as a patch
release with a `CHANGELOG.md` entry that names the issue once it is public.

## Reporting a vulnerability

Please do **not** open a public issue for a security problem.

1. Use GitHub's private vulnerability reporting on
   https://github.com/ygkali/panora/security/advisories/new, or
2. e-mail the maintainer at kompansebuyucu@proton.me with "Panora security" in
   the subject.

Include the Panora version (`panora-cli --version`), the desktop session
(`panora-doctor --json` output is ideal, it contains no clipboard content),
and steps to reproduce. You will get an acknowledgement within 7 days and a
fix or a decision within 90 days; you are credited in the release notes unless
you prefer not to be.

## What is in scope

- Clipboard content being stored when it should not be: password manager
  flags (`x-kde-passwordManagerHint`, `ConcealedType`, ...), excluded
  applications, private mode, the metadata limits.
- Payloads or previews reaching disk, logs or the session bus unencrypted.
- The IPC socket or the GNOME bridge accepting requests from a peer that is
  not the user (or not GNOME Shell), or being made to allocate without bound.
- The systemd sandbox of `panod.service` being bypassed by Panora's own code.
- The GNOME Shell extension doing anything beyond Super+V, clipboard
  forwarding and the two helper methods.

## What is out of scope

Documented in ADR 0003 (`docs/adr/0003-security-model.md`): the kernel, swap
and core dumps, a malicious GNOME Shell extension, and a user session that is
already compromised. Panora does not open network connections, so there is no
remote attack surface in this release.

## Verifying a release

Every release on GitHub ships `SHA256SUMS`; when the maintainer's minisign key
is configured the sums are signed as `SHA256SUMS.minisig`. Check a package with

```sh
sha256sum -c SHA256SUMS --ignore-missing
```
