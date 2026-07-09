#!/usr/bin/env sh
# Run when a task is created or restored from archive.
#
# tsk picks a monitor that does not host a global workspace (e.g. workspace "1"
# stays on its current monitor) and sets TSK_ON_START_MONITOR before running
# this script. Override with on_start_monitor in .tsk/repo.toml if needed.
# TSK_TASK_HOOK is "create" or "restore" if you need different behavior.
# When container isolation is enabled, TSK_CONTAINER_ISOLATION=1 and
# TSK_CONTAINER_NAME are set — use `tsk task editor` so Cursor runs in Distrobox.
if [ -n "$TSK_PRIMARY_NON_GLOBAL_WORKSPACE" ] && [ -n "$TSK_ON_START_MONITOR" ] && command -v hyprctl >/dev/null 2>&1; then
  hyprctl dispatch focusmonitor "$TSK_ON_START_MONITOR" >/dev/null 2>&1 || true
  hyprctl dispatch focusworkspaceoncurrentmonitor "name:$TSK_PRIMARY_NON_GLOBAL_WORKSPACE" >/dev/null 2>&1 || true
fi

# Prefer tsk launchers (respect Distrobox isolation when enabled on the task).
if command -v tsk >/dev/null 2>&1; then
  exec tsk task editor "$TSK_TASK_ID"
fi

if [ "${TSK_CONTAINER_ISOLATION:-0}" = "1" ] && [ -n "$TSK_CONTAINER_NAME" ] && command -v distrobox >/dev/null 2>&1; then
  if command -v cursor >/dev/null 2>&1; then
    exec distrobox enter --name "$TSK_CONTAINER_NAME" --no-tty -- cursor "$TSK_TASK_REPO"
  fi
fi

if command -v cursor >/dev/null 2>&1; then
  exec cursor "$TSK_TASK_REPO"
fi
