---
name: use-tsk-cli
description: >
  Call the tsk CLI for tasks, handoffs, checkouts, install, and agent-pack
  install. Use when TSK_TASK_ID is set, the user mentions tsk / taskspaces /
  Hypr Taskspace, or when creating, splitting, or instructing a task. Works
  from any checkout — including ones not yet in tsk.
---

# Use `tsk`

Hypr Taskspace (`tsk` / daemon `tskd`) is the environment host: linked
checkouts, Hyprland taskspaces, overlay, Waybar. It does **not** replace
orch (plan → N lanes); orch calls tsk.

## Go-to commands

```bash
tsk task list
tsk task current
tsk task switch <name-or-id>
tsk task new <name> [--repo-path PATH] [--scratch] [--from REV|--from-current|--from-workspace NAME]
tsk task new <name> … --handoff FILE|-
tsk task handoff [id] [--validate]
tsk task instruct [id] --from FILE|-
tsk checkout add <suffix> [--from REV]
tsk task editor | terminal | browser
tsk agents install --global [--force]
```

## Handoff

- Path: `~/tsk-tasks/<id>/workspace/HANDOFF.md` (`$TSK_HANDOFF`)
- Write: `tsk task instruct` or `tsk task new … --handoff`
- Read skill: **read-handoff**

## Split / fork

- CLI: `--from-current` / `--from` / `--from-workspace`
- Omarchy overlay: **Alt+S** Split (optional Write HANDOFF)
- Sibling WC in same task: `tsk checkout add <suffix>` (default fork = current)

## Install agent skills

```bash
tsk agents install --global          # link pack into ~/.cursor and ~/.claude skills
tsk agents install --global --force  # replace stale links
```

Materializes `~/.local/share/tsk/pack` → checkout or `/usr/share/tsk/pack`.
Override with `TSK_PACK_DIR` / `TSK_SHARE_DIR`.

## Signals

| Env / path | Meaning |
|------------|---------|
| `TSK_TASK_ID` | Active/spawned task |
| `TSK_TASK_REPO` | Task checkout (open this in the editor) |
| `TSK_SOURCE_REPO` | Linked source when present |
| `TSK_HANDOFF` | HANDOFF.md path (only when the file exists) |
| `~/tsk-tasks/<id>/.tsk/orch.json` | Also an orch lane (orch marker) |

Missing daemon → many create/switch RPCs fail closed; start with  
`systemctl --user status tskd` / `tsk doctor`.
