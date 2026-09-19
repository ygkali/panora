## What

<!-- One paragraph: the user-visible change and the roadmap id, e.g. "UI-03: Shift+Enter pastes plain text". -->

## Why

<!-- The problem this solves; link the issue. -->

## How

<!-- Design notes a reviewer needs: files touched, trade-offs, anything that affects privacy, storage format or the IPC protocol. -->

## Checklist

- [ ] `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` pass
- [ ] Tests cover the change (unit and, if it touches the desktop, an e2e step)
- [ ] `CHANGELOG.md` has an entry under *Unreleased*
- [ ] No payload can reach logs, the bus or disk unencrypted; no network dependency added
- [ ] Docs updated (README, `docs/protocol-matrix.md`, man pages) where behaviour changed
