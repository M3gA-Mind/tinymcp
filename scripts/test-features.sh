#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
python3 scripts/test-e2e-version.py
cargo test --locked --workspace
cargo test --locked -p tinymcp --no-default-features --test library_feature_tests
cargo test --locked -p tinymcp --no-default-features --features tools,ui,server-http --test library_feature_tests
cargo test --locked --workspace --all-features
scripts/check-engine-deps.sh
case "${RUNNER_OS:-$(uname -s)}" in
  Windows|MINGW*|MSYS*|CYGWIN*) ;;
  *) python3 scripts/test-package-layout.py ;;
esac
