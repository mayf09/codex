#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir/../codex-rs"

linker_flag=""
case "$(uname -s)" in
  Linux)
    if command -v mold >/dev/null 2>&1; then
      linker_flag="-C link-arg=-fuse-ld=mold"
    elif command -v ld.lld >/dev/null 2>&1; then
      linker_flag="-C link-arg=-fuse-ld=lld"
    fi
    ;;
  Darwin)
    if command -v ld.lld >/dev/null 2>&1 || [ -x /opt/homebrew/opt/lld/bin/ld.lld ]; then
      linker_flag="-C link-arg=-fuse-ld=lld"
    fi
    ;;
esac

if [ -n "$linker_flag" ]; then
  export RUSTFLAGS="${RUSTFLAGS:-} $linker_flag"
  echo "using RUSTFLAGS='$RUSTFLAGS'"
fi

exec cargo build --profile release-local -p codex-cli "$@"
