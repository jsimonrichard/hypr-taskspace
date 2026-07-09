#!/usr/bin/env sh
# Run when a task is created or restored from archive.
#
# tsk picks a monitor that does not host a global workspace (e.g. workspace "1"
# stays on its current monitor) and sets TSK_ON_START_MONITOR before running
# this script. Override with on_start_monitor in .tsk/repo.toml if needed.
# TSK_TASK_HOOK is "create" or "restore" if you need different behavior.
if [ -n "$TSK_PRIMARY_NON_GLOBAL_WORKSPACE" ] && [ -n "$TSK_ON_START_MONITOR" ] && command -v hyprctl >/dev/null 2>&1; then
  hyprctl dispatch focusmonitor "$TSK_ON_START_MONITOR" >/dev/null 2>&1 || true
  hyprctl dispatch focusworkspaceoncurrentmonitor "name:$TSK_PRIMARY_NON_GLOBAL_WORKSPACE" >/dev/null 2>&1 || true
fi

if command -v cursor >/dev/null 2>&1; then
  exec cursor "$TSK_TASK_REPO"
fi
