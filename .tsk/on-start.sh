#!/usr/bin/env sh
# Open the task checkout in Cursor when a new task is created.
#
# tsk picks a monitor that does not host a global workspace (e.g. workspace "1"
# stays on its current monitor) and sets TSK_ON_START_MONITOR before running
# this script. Override with on_start_monitor in .tsk/repo.toml if needed.
if [ -n "$TSK_TASK_WORKSPACE" ] && [ -n "$TSK_ON_START_MONITOR" ] && command -v hyprctl >/dev/null 2>&1; then
  hyprctl dispatch focusmonitor "$TSK_ON_START_MONITOR" >/dev/null 2>&1 || true
  hyprctl dispatch focusworkspaceoncurrentmonitor "name:$TSK_TASK_WORKSPACE" >/dev/null 2>&1 || true
fi

if command -v cursor >/dev/null 2>&1; then
  exec cursor "$TSK_TASK_REPO"
fi
