# Hypr Taskspace

Task-centric Hyprland control plane. Each task gets its own **taskspace** with named workspaces (`auth-fix-1`, `auth-fix-2`, …). The **default** (host) taskspace uses plain Hyprland workspaces **`1`–`10`** for everyday work.

Keybinds call `tsk` on your PATH. **Hyprland bindings** shown in this doc match the shipped Omarchy Lua (`omarchy.lua`) that `tsk install omarchy` `dofile`s from `~/.config/hypr/bindings.lua`; non-Omarchy Hyprland can still source `bindings.conf`. Runtime state lives in `~/.local/share/tsk/`. Templates live under `/usr/share/tsk/` (pacman) or `~/.local/share/tsk/` (cargo / from source).

## Prerequisites

- Hyprland (`hyprctl` on PATH)
- Rust toolchain — only if building from source ([rustup](https://rustup.rs/))

Optional:

- `distrobox` + Podman (or Docker) — **experimental** per-task container isolation (`tsk task new … --container`)

## Install

| | Pacman | Cargo / from source |
|---|--------|---------------------|
| CLI | `/usr/bin/tsk` | `~/.cargo/bin/tsk` |
| Share templates + Omarchy plugin | `/usr/share/tsk/` | `~/.local/share/tsk/` via script |
| Runtime data | `~/.local/share/tsk/` | same |
| systemd | packaged unit | `scripts/install-systemd.sh` |

**Arch (pacman):**

```bash
cd packaging/arch && makepkg -si
systemctl --user enable --now tskd.service
```

Then run `tsk install omarchy` (Lua bindings + bar plugin + menu launch prefix). Non-Omarchy Hyprland can source `/usr/share/tsk/hypr/bindings.conf` and optionally merge Waybar snippets under `/usr/share/tsk/waybar/`.

**Cargo / from source:**

```bash
cargo install --path crates/tsk-cli
scripts/install-user-share.sh
scripts/install-systemd.sh
```

Wire Hyprland to `~/.local/share/tsk/` the same way (`tsk install omarchy` on Omarchy).

**Omarchy** (Lua bindings, `tsk.taskspace` bar widget, cloned menu → `tsk launch`):

```bash
tsk install all                      # Omarchy + Chromium, whichever are present
# or individually:
tsk install omarchy
tsk install chromium
tsk doctor
```

Full steps, config examples, and uninstall: **[docs/install.md](docs/install.md)** · packaging paths: **[docs/packaging.md](docs/packaging.md)**

## Daily use

### Task manager (TUI)

The task manager is a **ratatui** terminal UI for creating, switching, and archiving tasks without memorizing CLI flags.

**Open it:**

| Method | Command / binding |
|--------|-------------------|
| Hyprland keybind | **SUPER+Tab** (default in shipped Lua / `bindings.conf`) |
| Omarchy bar | click the task label |
| Waybar (legacy) | click the task label |
| New terminal window | `tsk task tui-launch` |
| Current terminal | `tsk task tui` |

`tsk task tui-launch` (and **SUPER+Tab**) spawn a floating terminal (`org.tsk.task-tui`). `tsk task tui` runs in whatever terminal you are already in.

**Panels** — use **Tab** / **Shift+Tab**, or **h** / **l** / arrow keys to move between:

| Panel | Purpose |
|-------|---------|
| **Tasks** | Active tasks grouped by repo, plus **host → default taskspace** at the top |
| **Repos** | Registered git/jj checkouts used when creating tasks |
| **Archived** | Archived tasks (restore or delete) |

The current task is marked with **●**. Select **default taskspace** under **host** and press **Enter** to return to everyday Hyprland workspaces `1`–`10` (same as **SUPER+H** when using the default bindings).

**Tasks panel**

| Key | Action |
|-----|--------|
| ↑ / ↓ or **j** / **k** | Move selection |
| **Enter** | Switch to selected task (or default taskspace) |
| **n** | New task |
| **e** | Rename selected task label |
| **d** | Archive selected task |
| **D** | Delete selected task (with confirmation) |
| **R** | Refresh list |
| **q** / **Esc** | Quit |

**Archived panel** — **r** restore, **e** rename, **D** delete; other keys same as Tasks (except **d** / **n**).

**Repos panel**

| Key | Action |
|-----|--------|
| **n** | Browse directories and register a checkout at the current path (**Ctrl+Enter** / **Ctrl+y** to register) |
| **d** | Remove selected repo from the registry |
| **R** | Refresh |

**New task flow** — **n** on Tasks → pick a registered repo or **No repo (scratch workspace)** → enter a name → optionally toggle **worktree** (linked repos) and **Distrobox isolation** with **Space** → **Tab** between fields → confirm. Container tasks show setup progress in the TUI; on success the TUI switches to the new task and closes.

Creating, switching, and archiving tasks requires `tskd` to be running (`systemctl --user status tskd.service`). The TUI shows a warning banner when the daemon is down.

### CLI

Most task operations are also available from the CLI:

```bash
tsk task new my-feature              # git/jj from cwd (or scratch if none)
tsk task new notes --scratch         # empty workspace under the task home
tsk task new fix --repo-path /path/to/checkout
tsk task new main --no-worktree      # use the main checkout (no worktree)
tsk task new iso --container         # experimental Distrobox isolation
tsk task list
tsk task switch my-feature
tsk task rename my-feature "Auth Fix v2"
tsk task archive my-feature
tsk task restore my-feature
tsk task terminal                    # shell in the task checkout (Distrobox when --container)
tsk task editor                      # Cursor/VS Code (Distrobox when isolation is on)
tsk task browser                     # browser (Distrobox when isolation is on)
```

There is **experimental** support for container isolation with Distrobox: pass `--container` on the CLI or enable **Distrobox isolation** in the new-task form. Terminals, editor, and browser then launch via `distrobox enter`. Image defaults live under `[distrobox]` in `~/.config/tsk/config.toml`.

Task homes live under `~/tsk-tasks/<id>/`. Linked checkouts are at `~/tsk-tasks/<id>/workspace/<repo-name>` (scratch tasks use the `workspace/` directory itself). Optional checkout settings live in `.tsk/repo.toml`.

On create/restore, tsk runs `.tsk/on-start.sh` (opens the editor via `tsk task editor` by default). See **[docs/cursor.md](docs/cursor.md)**.

```bash
tsk repo add                         # register cwd
tsk repo list
tsk repo root                        # detected git/jj root for cwd
```

### Keybindings (Hyprland)

These match the defaults in `share/hypr/omarchy.lua` (Omarchy) and `share/hypr/bindings.conf` (legacy Hyprland `.conf`). `tsk install omarchy` `dofile`s the Lua file from `~/.config/hypr/bindings.lua` after unbinding Omarchy workspace and browser keys. Pacman and manual installs use the same templates from `/usr/share/tsk/hypr/` or `~/.local/share/tsk/hypr/`; remap freely, but the underlying commands stay the same (`tsk task tui-launch`, `tsk workspace switch 3`, `tsk launch chromium.desktop`, …).

| Action | Default binding |
|--------|---------|
| Task manager | **SUPER+Tab** (or bar task label) |
| Workspace 1–9 / 10 in current taskspace | **SUPER+1..9**, **SUPER+0** |
| Move window to workspace 1–10 | **SUPER+Shift+1..9 / 0** |
| Previous / next workspace | **SUPER+[** / **SUPER+]** (also trackpad swipe) |
| Default / host taskspace | **SUPER+H** or TUI → **host → default taskspace** |
| Task-aware terminal | **SUPER+Return** |
| Editor / browser (task) | **SUPER+E** / **SUPER+B** |
| Chromium via `tsk launch` | **SUPER+Shift+B**, **SUPER+Shift+Return** |
| Chromium private | **SUPER+Shift+Alt+B** |

Default and task taskspaces both use **10** slots so keybinds feel the same. Change the count with `workspace_count` under `[default]` in `~/.config/tsk/config.toml`.

**SUPER+Space** Apps on Omarchy go through a cloned `omarchy.menu` whose app-launch line calls `tsk launch <id>.desktop` (not Walker). Omarchy menu updates may require re-running `tsk install omarchy` to re-apply that patch.

Chromium in a taskspace (`tsk launch chromium.desktop`, **SUPER+B**, or `tsk task browser`) uses the **host profile** so extensions and logins (password manager, etc.) are shared. Set `isolate_profile = true` under `[browser]` for a blank per-task `--user-data-dir`. Tabs are snapshotted automatically; the first Chromium launch in a taskspace with no window reopens that snapshot (including after archive/restore).

### Useful commands

```bash
tsk doctor
tsk status
tsk taskspace default                # same as SUPER+H (default bindings)
tsk windows                          # list windows + task association
tsk windows restore                  # move windows back to home workspaces
tsk daemon status
```

## Troubleshooting

```bash
tsk doctor
tsk taskspace default
systemctl --user status tskd.service
hyprctl reload                       # after changing Lua or Hypr source lines
```

If the Omarchy bar widget is stale, run `omarchy-shell shell call tsk.taskspace refresh` (or `tsk doctor`). Waybar CFFI is optional/legacy for non-Omarchy Hyprland.

## More documentation

| Doc | For |
|-----|-----|
| [docs/install.md](docs/install.md) | Production install, Omarchy plugin / Lua / launch intercept |
| [docs/packaging.md](docs/packaging.md) | Arch package layout and AUR notes |
| [docs/dev.md](docs/dev.md) | Developing tsk itself (dev session, e2e) |
| [docs/cursor.md](docs/cursor.md) | Cursor / on-start hooks |
