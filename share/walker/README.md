# Walker / Elephant integration

Walker (via [Elephant](https://github.com/antonycourtney/elephant)) launches applications through a configurable prefix. TSK hooks that prefix so every Walker launch gets taskspace environment variables and uses the same terminal/browser/editor integrations as `tsk task`.

## Install

Omarchy preset (includes Walker):

```bash
tsk install omarchy
```

Walker only:

```bash
tsk install walker
```

Dry run:

```bash
tsk install walker --dry-run
```

This patches `~/.config/elephant/elephant.toml` (root-level keys, before any `[section]` header):

```toml
auto_detect_launch_prefix = false
launch_prefix = "/usr/bin/tsk walker exec --"
terminal_cmd = "/usr/bin/tsk walker terminal --"

[provider_hosts]
```

Elephant is restarted automatically when `elephant.service` is active.

## How it works

| Elephant setting | TSK command | Behavior |
|------------------|-------------|----------|
| `launch_prefix` | `tsk walker exec -- <app>` | Resolves active taskspace from Hyprland + SQLite, sets `TSK_*` env, routes terminals/browsers/editors through `tsk task` integrations |
| `terminal_cmd` | `tsk walker terminal -- [cmd…]` | Empty args → task terminal; with args → run command in task-scoped terminal |

### Routing (`walker exec`)

- **Terminals** (alacritty, kitty, foot, ghostty, etc.) → `tsk task terminal`
- **Browsers** (chromium, firefox, …) → `tsk task browser` when in a task; otherwise browser with taskspace env
- **Editors** (cursor, code) → `tsk task editor` when in a task; otherwise editor with taskspace env
- **Everything else** → `uwsm app <desktop-id>` (or direct exec) with task env and task repo as cwd

## Verify

```bash
tsk integration status
tsk doctor
```

From a task taskspace, open Walker and launch terminal, browser, and editor — confirm `TSK_TASK_ID` (and related vars) in the spawned process and correct working directory.

## Uninstall

Restore from install backup (if manifest exists):

```bash
# Manual: copy backup from ~/.local/share/tsk/install/walker/backups/<timestamp>/elephant.toml
```

Or remove managed lines from `elephant.toml` (lines tagged `# tsk-managed`).

## Manual setup

If you cannot run install, set Elephant config yourself using the paths from:

```bash
tsk integration status
```

Restart Elephant / Walker after editing:

```bash
systemctl --user restart elephant.service
# or: omarchy-restart-walker
```
