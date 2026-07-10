#!/usr/bin/env bash
# Close floating terminals opened by tsk latency / spawn benchmarks.
# Safe to run when no matching windows exist.
set -euo pipefail

if ! command -v hyprctl >/dev/null 2>&1; then
  exit 0
fi

if ! hyprctl clients -j >/dev/null 2>&1; then
  exit 0
fi

hyprctl clients -j | python3 -c "
import json, subprocess, sys

clients = json.load(sys.stdin)
closed = 0
for c in clients:
    cls = c.get('class', '') or ''
    title = c.get('title', '') or ''
    if cls in ('org.tsk.task-tui', 'org.tsk.task-terminal'):
        match = True
    elif title in ('tsk tasks', 'terminal'):
        match = True
    elif title.startswith('[') and title.endswith('] terminal'):
        match = True
    else:
        match = False
    if not match:
        continue
    addr = c['address']
    hexaddr = addr if str(addr).startswith('0x') else f'0x{addr}'
    subprocess.run(
        ['hyprctl', 'dispatch', 'closewindow', f'address:{hexaddr}'],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    closed += 1
    print(f'closed {hexaddr}  class={cls!r}  title={title!r}', file=sys.stderr)

if closed:
    print(f'Closed {closed} tsk test terminal(s).', file=sys.stderr)
"
