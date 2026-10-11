#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
planner="$repo_root/scripts/release-version-plan.sh"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT

git -C "$scratch" init -q
git -C "$scratch" config user.name "Release plan test"
git -C "$scratch" config user.email "release-plan-test@example.invalid"
mkdir -p "$scratch/crates/example"
cat > "$scratch/Cargo.toml" <<'EOF'
[workspace]
members = ["crates/example"]

[workspace.package]
version = "0.13.0"
EOF
cat > "$scratch/crates/example/Cargo.toml" <<'EOF'
[package]
name = "example"
version.workspace = true
EOF
git -C "$scratch" add Cargo.toml crates/example/Cargo.toml
git -C "$scratch" commit -qm "fixture"

plan="$(cd -- "$scratch" && "$planner" minor 0.13.0)"
[[ "$plan" == "prepare-version-pr 0.14.0 v0.14.0 create" ]]

plan="$(cd -- "$scratch" && "$planner" current 0.13.0)"
[[ "$plan" == "publish-current 0.13.0 v0.13.0 create" ]]

git -C "$scratch" tag -a v0.13.0 -m "matching release" HEAD
plan="$(cd -- "$scratch" && "$planner" current 0.13.0)"
[[ "$plan" == "publish-current 0.13.0 v0.13.0 existing" ]]

git -C "$scratch" tag -a v0.14.0 -m "mismatched release" HEAD
if (cd -- "$scratch" && "$planner" current 0.14.0) >/dev/null 2>&1; then
  echo "current mode accepted a tag whose source version is mismatched" >&2
  exit 1
fi

echo "release version planning checks passed"
