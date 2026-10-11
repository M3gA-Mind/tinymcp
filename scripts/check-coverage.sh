#!/usr/bin/env bash
set -euo pipefail

# Reuse the established per-file 90% coverage gate from the repository root.
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$repo_root"
exec bash "$repo_root/.github/scripts/check-file-coverage.sh" 90 coverage.json
