#!/usr/bin/env bash
set -euo pipefail

# Build the real cdylib and exercise it through TinyBus's native module loader.
cargo build --package tinymcp --lib

case "${RUNNER_OS:-$(uname -s)}" in
  Windows|MINGW*|MSYS*|CYGWIN*) module_path="target/debug/tinymcp.dll" ;;
  macOS|Darwin) module_path="target/debug/libtinymcp.dylib" ;;
  *) module_path="target/debug/libtinymcp.so" ;;
esac

if [[ ! -f "$module_path" ]]; then
  echo "expected built TinyMCP module at $module_path" >&2
  exit 1
fi

cargo run --package tinymcp --example verify_module -- "$module_path"
