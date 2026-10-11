#!/usr/bin/env bash
set -euo pipefail

# Keep the shared module-CI input rooted under scripts/ while reusing the
# repository's established per-file 90% coverage gate.
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
exec bash "$repo_root/.github/scripts/check-file-coverage.sh" 90 coverage.json
