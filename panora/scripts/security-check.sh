#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
export RUSTFLAGS="-D warnings"
. "$HOME/.cargo/env" 2>/dev/null || true
printf '%s\n' '[1/8] format'
cargo fmt --all -- --check
printf '%s\n' '[2/8] clippy'
cargo clippy --workspace --all-targets -- -D warnings
printf '%s\n' '[3/8] tests'
cargo test --workspace
printf '%s\n' '[4/8] application unsafe-code scan'
if grep -RInE --include='*.rs' '\bunsafe\b' crates; then
  echo 'unsafe Rust found in application crates' >&2
  exit 1
fi
printf '%s\n' '[5/8] network dependency surface'
# This used to run `cargo tree -p panora-sync`, a package that does not exist in
# the workspace. cargo failed, grep read nothing and exited 1, so the `if` was
# false and the gate reported success without ever inspecting a dependency.
# Resolve the tree for the whole workspace instead, and fail loudly if the tree
# itself cannot be produced.
TREE="$(cargo tree --workspace --edges normal)" || {
  echo 'cannot resolve dependency tree; network surface unverified' >&2
  exit 1
}
if printf '%s\n' "$TREE" | grep -E 'iroh|quinn|reqwest|hyper|rustls|native-tls|trust-dns|hickory'; then
  echo 'network dependency unexpectedly present in the workspace' >&2
  exit 1
fi
printf '%s\n' '[6/8] GNOME extension static security checks'
node scripts/extension-security-check.mjs
printf '%s\n' '[7/8] Cargo.lock reproducibility'
cargo metadata --locked --no-deps --format-version 1 >/dev/null
printf '%s\n' '[8/8] dependency advisory checks'
if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit
elif [ "${CI:-}" = "true" ]; then
  echo 'cargo-audit is required in CI but is not installed' >&2
  exit 1
else
  echo 'cargo-audit unavailable locally; CI must run the RustSec job' >&2
fi
if command -v cargo-deny >/dev/null 2>&1; then
  cargo deny check
elif [ "${CI:-}" = "true" ]; then
  echo 'cargo-deny is required in CI but is not installed' >&2
  exit 1
else
  echo 'cargo-deny unavailable locally; CI must run the license/advisory job' >&2
fi
echo 'security-check: PASS'
