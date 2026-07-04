#!/usr/bin/env bash
# Install share templates + Waybar module to ~/.local/share/tsk (cargo / from-source).
# Pacman users: use packaging/arch/PKGBUILD instead — no script needed.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export TSK_WORKSPACE="$ROOT"
cd "$ROOT"

if [[ -x "$ROOT/target/release/tsk" ]]; then
  TSK="$ROOT/target/release/tsk"
else
  echo "Building tsk + tsk-waybar (release)..."
  cargo build -p tsk-cli -p tsk-waybar --release
  TSK="$ROOT/target/release/tsk"
fi

exec "$TSK" dev install share --prod "$@"
