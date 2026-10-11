#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
archive="" verifier_bin=""
while (($#)); do
  case "$1" in
    --archive) archive="${2:?archive path required}"; shift 2 ;;
    --verifier-bin) verifier_bin="${2:?verifier binary path required}"; shift 2 ;;
    *) echo "usage: verify-module.sh --archive <module-archive> [--verifier-bin <path>]" >&2; exit 2 ;;
  esac
done
[[ -n "$archive" ]] || { echo "usage: verify-module.sh --archive <module-archive> [--verifier-bin <path>]" >&2; exit 2; }
archive="$(cd "$(dirname "$archive")" && pwd -P)/$(basename "$archive")"
cd "$root"
target_root="${CARGO_TARGET_DIR:-$root/target}"
[[ "$target_root" == /* ]] || target_root="$root/$target_root"
mkdir -p "$target_root"
target_root="$(cd "$target_root" && pwd -P)"
[[ "$target_root" == "$root"/* ]] || { echo "CARGO_TARGET_DIR must stay inside the repository" >&2; exit 2; }
work="$target_root/tinymcp-verify-$$"
rm -rf "$work"
mkdir -p "$work"
chmod 700 "$work"
trap 'rm -rf "$work"' EXIT
case "$archive" in
  *.tar.gz)
    members="$(tar -tzf "$archive")"
    while IFS= read -r member; do
      case "$member" in
        /*|../*|*/../*|*/..) echo "archive member escapes the extraction directory: $member" >&2; exit 2 ;;
      esac
    done <<<"$members"
    if tar -tvzf "$archive" | awk 'substr($0, 1, 1) !~ /^[-d]$/ { exit 1 }'; then
      tar -xzf "$archive" -C "$work"
    else
      echo "archive contains a link or special file" >&2
      exit 2
    fi
    ;;
  *) echo "unsupported module archive: $archive" >&2; exit 2 ;;
esac
case "$(uname -s)" in
  Darwin) library="$work/libtinymcp_module.dylib" ;;
  Linux) library="$work/libtinymcp_module.so" ;;
  *) echo "verify-module.sh requires a Unix runner" >&2; exit 1 ;;
esac
test -f "$work/modules.toml"
if [[ -n "$verifier_bin" ]]; then
  [[ "$verifier_bin" == /* ]] || verifier_bin="$root/$verifier_bin"
  [[ "$verifier_bin" == "$root"/* && -f "$verifier_bin" && -x "$verifier_bin" && ! -L "$verifier_bin" ]] || {
    echo "verifier binary must be an executable repository file" >&2
    exit 2
  }
  "$verifier_bin" "$library"
else
  cargo run --locked --release --manifest-path Cargo.toml --package tinymcp --example verify_module -- "$library"
fi
