#!/usr/bin/env bash
set -euo pipefail

if command -v dnf >/dev/null 2>&1; then
  dnf install -y gcc gcc-c++ cmake curl git gzip jq make openssl-devel perl tar util-linux
elif command -v pacman >/dev/null 2>&1; then
  # Distro builds run inside a nested container on hosted runners. Landlock
  # cannot be applied there, so disable only pacman's download filesystem
  # sandbox; its syscall sandbox remains enabled.
  pacman -Syu --disable-sandbox-filesystem --noconfirm base-devel cmake curl git gzip jq openssl perl util-linux
else
  echo "unsupported distro package manager" >&2
  exit 2
fi
