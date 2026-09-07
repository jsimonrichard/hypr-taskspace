#!/bin/bash
# hyprctl reload, tskd restart, and Omarchy plugin refresh after a tsk
# package transaction. Invoked as root from the alpm PostTransaction hook.
#
# Plugin files are copied even when the shell cannot restart. A locked
# session (or any other `omarchy restart shell` failure) fails this hook
# with a warning — the package is already installed, but the running
# overlay is still the previous QML.

shopt -s nullglob

if ! command -v hyprctl >/dev/null 2>&1; then
  exit 0
fi
if ! command -v runuser >/dev/null 2>&1; then
  exit 0
fi

plugin_src=/usr/share/tsk/omarchy-plugin
omarchy_path=${OMARCHY_PATH:-/usr/share/omarchy}
hook_failed=0
refreshed_uids=" "

# Hyprland sources /usr/share/tsk/hypr directly. omarchy-shell does not —
# it loads ~/.config/omarchy/plugins/tsk.taskspace, a copy from
# `tsk install omarchy`.
refresh_omarchy_plugin() {
  local user=$1 runtime=$2 sig=$3
  local home dest overlay tsk_cmd src name group out rc

  home=$(getent passwd "$user" | cut -d: -f6) || return 0
  [[ -n $home ]] || return 0
  dest="$home/.config/omarchy/plugins/tsk.taskspace"
  [[ -d $dest && -d $plugin_src ]] || return 0

  overlay=0
  [[ -f $dest/Taskspace.qml ]] && overlay=1

  tsk_cmd=/usr/bin/tsk
  if [[ -f $dest/BarWidget.qml ]]; then
    local stamped
    stamped=$(sed -n 's/.*tskCmd: "\([^"]*\)".*/\1/p' "$dest/BarWidget.qml" | head -n1)
    [[ -n $stamped ]] && tsk_cmd=$stamped
  fi

  group=$(id -gn "$user" 2>/dev/null) || group=$user

  for src in "$plugin_src"/*; do
    [[ -f $src ]] || continue
    name=$(basename "$src")
    if [[ $overlay -eq 0 && ( $name == Taskspace.qml || $name == TaskspaceModel.js || $name == manifest.json ) ]]; then
      continue
    fi
    tmp=$(mktemp)
    sed "s|@TSK_CMD@|${tsk_cmd}|g" "$src" >"$tmp" || true
    install -o "$user" -g "$group" -m 644 "$tmp" "$dest/$name" || true
    rm -f "$tmp"
  done

  out=$(runuser -u "$user" -- env \
    PATH="/usr/bin:/usr/local/bin" \
    XDG_RUNTIME_DIR="$runtime" \
    HYPRLAND_INSTANCE_SIGNATURE="$sig" \
    DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus" \
    OMARCHY_PATH="$omarchy_path" \
    omarchy restart shell 2>&1)
  rc=$?
  if [[ $rc -ne 0 ]]; then
    echo "hypr-taskspace: warning: Omarchy shell was not restarted." >&2
    echo "Plugin files were updated. After unlock, run: omarchy restart shell" >&2
    if [[ -n $out ]]; then
      echo "$out" >&2
    fi
    return "$rc"
  fi
}

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

  runuser -u "$user" -- env \
    XDG_RUNTIME_DIR="$runtime" \
    DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus" \
    systemctl --user try-restart tskd.service >/dev/null 2>&1 || true

  if [[ $refreshed_uids == *" $uid "* ]]; then
    continue
  fi
  refreshed_uids+="$uid "
  if ! refresh_omarchy_plugin "$user" "$runtime" "$sig"; then
    hook_failed=1
  fi
done

exit "$hook_failed"
