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
printf '%s\n' '[5/8] sync dependency surface'
if cargo tree -p panora-sync | grep -E 'iroh|quinn|reqwest|hyper|tokio.*net'; then
  echo 'network dependency unexpectedly present in panora-sync v1' >&2
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
