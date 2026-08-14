# Walker / Elephant integration

Walker (via [Elephant](https://github.com/antonycourtney/elephant)) launches applications through a configurable prefix. TSK hooks that prefix so every Walker launch gets taskspace environment variables and uses the same terminal/browser/editor integrations as `tsk task`.

## Install

Omarchy preset (includes Walker):

```bash
tsk install all
# or: tsk install omarchy
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

The selected app is preserved. Walker → VS Code opens VS Code; Walker → Firefox opens Firefox. TSK injects task env and extra args for that binary (task checkout for editors, Chromium `--new-window` on the task workspace). `tsk task editor` / `tsk task browser` with no selected app still use the preferred fallbacks (Cursor, then VS Code; configured browser).

- **Terminals** (alacritty, kitty, foot, ghostty, etc.) → task terminal using the selected emulator
- **Browsers** (chromium, firefox, …) → selected browser; Chromium-family shares the host profile by default (extensions and logins). Set `[browser].isolate_profile = true` in `~/.config/tsk/config.toml` for a per-task `--user-data-dir`. The first Chromium launch after restoring a task reopens that task's saved tabs.
- **Editors** (cursor, code) → selected editor, opening the task checkout when in a task
- **Everything else** → `uwsm app -- <parsed Exec argv>` (or desktop id if Exec is missing) with task env and task repo as cwd. `--` keeps app flags such as `--no-sandbox` from being parsed as uwsm options; launching via argv also avoids uwsm rejecting desktop files that use both `%u` and `%U`.

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
