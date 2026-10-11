#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
python3 scripts/test-e2e-version.py
cargo test --locked --workspace
cargo test --locked --workspace --all-features

set -euo pipefail
# A static-link build has Rust entries but no dynamic ABI symbols.
cargo build --package tinymcp --lib
cargo run --package tinymcp --example verify_module -- \
  target/debug/libtinymcp.so

set -euo pipefail
forbidden="$(cargo tree -p tinymcp-bus -e normal,build --prefix none \
  | grep -Ei 'tinybus|tokio|reqwest|ureq|hyper|rusqlite|git2' || true)"
if [ -n "$forbidden" ]; then
  echo "tinymcp-bus pulled in a dependency its manifest forbids:" >&2
  echo "$forbidden" >&2
  echo >&2
  echo "The contract is what a host compiles against. It must stay free" >&2
  echo "of transports, async runtimes, HTTP clients and native libraries." >&2
  exit 1
fi
case "${RUNNER_OS:-$(uname -s)}" in
  Windows|MINGW*|MSYS*|CYGWIN*) ;;
  *) python3 scripts/test-package-layout.py ;;
esac
