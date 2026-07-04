#!/usr/bin/env bash
# Enable the tsk user daemon. Pacman installs the unit to /usr/lib/systemd/user/.
set -euo pipefail

PACKAGED="/usr/lib/systemd/user/tskd.service"
USER_UNIT="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/tskd.service"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ -f "$PACKAGED" ]]; then
  echo "Using packaged unit: $PACKAGED"
  systemctl --user daemon-reload
  systemctl --user enable --now tskd.service
  exit 0
fi

if [[ ! -f "$USER_UNIT" ]]; then
  mkdir -p "$(dirname "$USER_UNIT")"
  sed "s|@TSK_CMD@|tsk|g" "$ROOT/share/systemd/tskd.service" >"$USER_UNIT"
  echo "Installed $USER_UNIT"
fi

systemctl --user daemon-reload
systemctl --user enable --now tskd.service
echo "tskd.service enabled and started."
