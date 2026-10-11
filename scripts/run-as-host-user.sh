#!/usr/bin/env bash
set -euo pipefail

uid="${1:-}" gid="${2:-}"
if [[ ! "$uid" =~ ^[0-9]+$ || ! "$gid" =~ ^[0-9]+$ || $# -lt 3 ]]; then
  echo "usage: run-as-host-user.sh UID GID COMMAND [ARG ...]" >&2
  exit 2
fi
shift 2
if [[ "$uid" == "$(id -u)" && "$gid" == "$(id -g)" ]]; then
  exec "$@"
fi
command -v setpriv >/dev/null 2>&1 || {
  echo "setpriv is required to verify modules as the workspace owner" >&2
  exit 2
}
exec setpriv --reuid="$uid" --regid="$gid" --clear-groups -- "$@"
