#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <patch|minor|major|current> <current-version>" >&2
  exit 64
fi

bump="$1"
current_version="$2"
if [[ ! "$current_version" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+)$ ]]; then
  echo "invalid current version: $current_version" >&2
  exit 64
fi

major="${BASH_REMATCH[1]}"
minor="${BASH_REMATCH[2]}"
patch="${BASH_REMATCH[3]}"

case "$bump" in
  current)
    next_version="$current_version"
    action="publish-current"
    ;;
  patch)
    patch=$((patch + 1))
    next_version="${major}.${minor}.${patch}"
    action="prepare-version-pr"
    ;;
  minor)
    minor=$((minor + 1))
    patch=0
    next_version="${major}.${minor}.${patch}"
    action="prepare-version-pr"
    ;;
  major)
    major=$((major + 1))
    minor=0
    patch=0
    next_version="${major}.${minor}.${patch}"
    action="prepare-version-pr"
    ;;
  *)
    echo "unsupported bump: $bump" >&2
    exit 64
    ;;
esac

tag="v${next_version}"
if git rev-parse --verify --quiet "refs/tags/${tag}" >/dev/null; then
  if [[ "$action" == "prepare-version-pr" ]]; then
    echo "tag ${tag} already exists; refusing to prepare another version PR" >&2
    exit 1
  fi
  tagged_version="$(
    git show "${tag}:Cargo.toml" \
      | sed -n '/^\[workspace\.package\]/,/^\[/ s/^version = "\([^"]*\)"/\1/p' \
      | head -n 1
  )"
  if [[ "$tagged_version" != "$current_version" ]]; then
    echo "tag ${tag} does not contain version ${current_version}" >&2
    exit 1
  fi
  tag_action="existing"
else
  tag_action="create"
fi

printf '%s %s %s %s\n' "$action" "$next_version" "$tag" "$tag_action"
