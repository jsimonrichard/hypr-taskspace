# lae — Local Agentic Environment

Task-centric Hyprland + Distrobox control plane. Each task gets its own **taskspace** with named **workspaces** (`auth-fix-1`, `auth-fix-2`, …), an isolated Distrobox container, and a repo clone. The **default** taskspace uses plain Hyprland workspace names **`1`–`10`** for everyday host work.

## Prerequisites

- Hyprland (Omarchy or similar)
- Python ≥ 3.11
- `distrobox` + Podman (optional but recommended for task containers)
- Walker + Elephant (Omarchy ships these)
- Git

---

## Install

### 1. Install the CLI (editable)

From the repo root:

```bash
cd ~/Desktop/local-agentic-env
pip install -e .
```

This is an **editable install**: Python code changes under `src/lae/` take effect immediately. Re-run the integration steps below when files under `share/` change (keybinds, Waybar modules, Walker menu).

Verify the CLI:

```bash
lae --help
lae status
```

On first run, lae creates `~/.config/lae/config.toml` and `~/.local/share/lae/state.db`.

### 2. Integrate with Hyprland, Walker, and Waybar

One command installs everything and reloads Hyprland, Walker, and Waybar:

```bash
lae install all --dry-run   # optional: preview changes
lae install all
```

Or install step by step (each step reloads what it changed):

```bash
lae install hypr --dry-run   # optional: preview changes
lae install hypr

lae install waybar --dry-run   # optional
lae install waybar
```

Hyprland install copies templates to `~/.local/share/lae/`, adds a `source` line to your Hyprland config (with a full-file backup), and installs the Walker task menu. Waybar install patches your Waybar config (with backup).

| Artifact | Location |
|----------|----------|
| Hyprland keybinds + Omarchy unbinds | `~/.local/share/lae/hypr/bindings.conf`, `unbind-omarchy.conf` |
| `lae` wrapper (for keybinds / Walker) | `~/.local/share/lae/bin/lae` |
| Walker menu helper | `~/.local/share/lae/bin/lae-task-menu-json` |
| Elephant menu | `~/.config/elephant/menus/lae_tasks.lua` |
| Config backup | `~/.local/share/lae/install/hypr/backups/<timestamp>/` |

Waybar install replaces the stock workspace indicator with the current task name and scoped desktop buttons. Backs up `~/.config/waybar/config.jsonc` before patching.

### 3. Check installation

```bash
lae doctor
lae install status
```

---

## Daily use

### Create and enter a task

```bash
lae task new my-feature --repo git@github.com:you/project.git
lae task switch my-feature
lae task terminal          # Distrobox terminal (SUPER+T)
```

### Switch back to normal host desktops

Open the **task menu** (Waybar task label, SUPER+Tab, or `lae task menu`) and choose **default**, or:

```bash
lae context default        # SUPER+H
```

### Keybindings (after `lae install hypr`)

| Action | Binding |
|--------|---------|
| Task menu (Walker) | Click task name in Waybar, **SUPER+Tab**, or `lae task menu` |
| Workspace 1–9 / 10 within current taskspace | **SUPER+1..9**, **SUPER+0** (= workspace 10) |
| Task terminal (Distrobox) | **SUPER+T** |
| Default / host taskspace | **SUPER+H** or Walker → **default** |
| Global escape hatch (all Hyprland workspaces) | **SUPER+Escape** |
| Host terminal | **SUPER+Return** (your existing Omarchy bind — unchanged) |

SUPER+Space remains the normal Walker app launcher, not the task menu.

Default taskspace supports **10** Hyprland workspaces (`1`–`10`). Waybar updates **immediately** on taskspace/workspace changes (signal-driven, not 1s polling). Task taskspaces still use 3 scoped workspaces. Set `workspace_count = 10` under `[default]` in `~/.config/lae/config.toml`.

CLI: `lae taskspace` and `lae workspace` (legacy aliases: `context`, `desktop`).

---

## Update after pulling changes

```bash
pip install -e .              # if dependencies changed
lae install all               # refresh templates and reload everything
lae daemon start              # keeps keybinds fast; Waybar uses a shared cache either way
```

---

## Uninstall

Run in order. Integration uninstallers **restore your backed-up config files**; they do not delete task data unless you remove it manually.

### 1. Remove Hyprland and Waybar integration

```bash
lae uninstall waybar    # restores ~/.config/waybar/config.jsonc from backup
lae uninstall hypr      # restores ~/.config/hypr/hyprland.conf from backup
```

Each uninstall restores your backed-up config and reloads the affected component.

To keep lae-owned files under `~/.local/share/lae/hypr/` for inspection:

```bash
lae uninstall hypr --keep-files
```

### 2. Remove the Python package

```bash
pip uninstall local-agentic-env
```

### 3. Optional: remove task data and state

These are **not** removed by `lae uninstall` (your clones and state are preserved):

```bash
rm -rf ~/.local/share/lae/state.db    # task registry
rm -rf ~/lae-tasks/                   # repo clones and containers' home dirs
rm -rf ~/.config/lae/                 # user config
```

Distrobox containers (`lae-<task-id>`) must be removed separately if desired:

```bash
distrobox list
distrobox rm lae-my-feature
```

### Manual rollback (if uninstall fails)

Restore Hyprland config from the latest backup:

```bash
cp ~/.local/share/lae/install/hypr/backups/*/hyprland.conf ~/.config/hypr/hyprland.conf
hyprctl reload
```

---

## Troubleshooting

**Walker task menu is empty**

Elephant may not see `lae` on PATH. Re-run `lae install hypr` (reloads Walker automatically):

```bash
~/.local/share/lae/bin/lae-task-menu-json   # should list default + tasks
omarchy-restart-walker
systemctl --user restart elephant.service   # if still empty
```

**Check overall health**

```bash
lae doctor
lae status
```
