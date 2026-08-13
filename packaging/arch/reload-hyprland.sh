#!/bin/bash
# Best-effort hyprctl reload for every running Hyprland instance.
# Invoked as root from the alpm PostTransaction hook; never fail the install.

shopt -s nullglob

if ! command -v hyprctl >/dev/null 2>&1; then
  exit 0
fi
if ! command -v runuser >/dev/null 2>&1; then
  exit 0
fi

for socket in /run/user/*/hypr/*/.socket.sock; do
  [[ -S $socket ]] || continue

  inst_dir=$(dirname "$socket")
  sig=$(basename "$inst_dir")
  runtime=$(dirname "$(dirname "$inst_dir")")
  uid=$(basename "$runtime")
  [[ $uid =~ ^[0-9]+$ ]] || continue

  user=$(getent passwd "$uid" | cut -d: -f1) || continue
  [[ -n $user ]] || continue

  runuser -u "$user" -- env \
    XDG_RUNTIME_DIR="$runtime" \
    HYPRLAND_INSTANCE_SIGNATURE="$sig" \
    hyprctl reload >/dev/null 2>&1 || true
done

exit 0
