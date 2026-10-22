#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
python3 scripts/test-e2e-version.py
cargo test --locked --workspace
cargo test --locked -p tinymcp --no-default-features --test library_feature_tests
cargo test --locked -p tinymcp --no-default-features --features tools,ui,server-http --test library_feature_tests
cargo test --locked --workspace --all-features
default_tree="$(cargo tree -p tinymcp-bus -e normal,build --prefix none)"
storage_tree="$(cargo tree -p tinymcp-bus --features storage -e normal,build --prefix none)"
forbidden="$(printf '%s\n' "$default_tree" \
  | grep -Ei 'tinybus|tokio|reqwest|ureq|hyper|rusqlite|git2|data-encoding|sha2' || true)"
storage_forbidden="$(printf '%s\n' "$storage_tree" \
  | grep -Ei 'tinybus|reqwest|ureq|hyper|rusqlite|git2|data-encoding|sha2' || true)"
if [ -n "$storage_forbidden" ]; then
  forbidden="${forbidden:+$forbidden$'\n'}$storage_forbidden"
fi
if [ -n "$forbidden" ]; then
  echo "tinymcp-bus pulled in a dependency its manifest forbids:" >&2
  echo "$forbidden" >&2
  echo >&2
  echo "The contract must stay free of transports, async runtimes, HTTP clients and native libraries." >&2
  exit 1
fi
scripts/check-engine-deps.sh
case "${RUNNER_OS:-$(uname -s)}" in
  Windows|MINGW*|MSYS*|CYGWIN*) ;;
  *) python3 scripts/test-package-layout.py ;;
esac
