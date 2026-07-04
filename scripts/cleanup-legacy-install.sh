#!/usr/bin/env bash
# Remove user-local share copies superseded by /usr/share/tsk (pacman) or PATH-based CLI.
# Keeps runtime data: state.db, daemon.sock, and ~/.local/share/tsk/install/ manifests.
set -euo pipefail

DATA="${XDG_DATA_HOME:-$HOME/.local/share}/tsk"
SYSTEM_SHARE="/usr/share/tsk"
DRY_RUN="${1:-}"

run() {
  if [[ "$DRY_RUN" == "--dry-run" ]]; then
    echo "would: $*"
  else
    echo "remove: $*"
    rm -rf "$@"
  fi
}

if [[ ! -d "$DATA" ]]; then
  echo "No $DATA — nothing to clean."
  exit 0
fi

echo "Cleaning legacy install artifacts under $DATA"
echo "(keeping state.db, daemon.sock, install/ metadata)"
echo

# Old CLI copy (replaced by /usr/bin/tsk or ~/.cargo/bin/tsk)
if [[ -f "$DATA/bin/tsk" ]]; then
  run "$DATA/bin/tsk"
fi

# Duplicate share tree when system package is installed
if [[ -f "$SYSTEM_SHARE/hypr/bindings.conf" ]]; then
  for dir in hypr lib waybar; do
    [[ -e "$DATA/$dir" ]] && run "$DATA/$dir"
  done
  # Helpers shipped in /usr/share/tsk/bin/
  if [[ -d "$DATA/bin" ]]; then
    for helper in "$DATA/bin"/tsk-*; do
      [[ -e "$helper" ]] || continue
      run "$helper"
    done
    # Remove bin/ if empty
    if [[ "$DRY_RUN" != "--dry-run" ]] && [[ -d "$DATA/bin" ]] && [[ -z "$(ls -A "$DATA/bin" 2>/dev/null)" ]]; then
      run "$DATA/bin"
    fi
  fi
fi

# User systemd unit when packaged unit exists
PACKAGED_UNIT="/usr/lib/systemd/user/tskd.service"
USER_UNIT="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/tskd.service"
if [[ -f "$PACKAGED_UNIT" && -f "$USER_UNIT" ]]; then
  run "$USER_UNIT"
  if [[ "$DRY_RUN" != "--dry-run" ]]; then
    systemctl --user daemon-reload 2>/dev/null || true
  fi
fi

echo
echo "Done. Runtime data:"
ls -la "$DATA/" 2>/dev/null || true
