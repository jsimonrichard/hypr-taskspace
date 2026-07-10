# Hypr Taskspace

Task-centric Hyprland control plane. Each task gets its own **taskspace** with named workspaces (`auth-fix-1`, `auth-fix-2`, …). The **default** (host) taskspace uses plain Hyprland workspaces **`1`–`10`** for everyday work.

Keybinds call `tsk` on your PATH. Runtime state lives in `~/.local/share/tsk/`. Templates and the Waybar module live under `/usr/share/tsk/` (pacman) or `~/.local/share/tsk/` (cargo / from source).

## Prerequisites

- Hyprland (`hyprctl` on PATH)
- Rust toolchain — only if building from source ([rustup](https://rustup.rs/))

Optional:

- `distrobox` + Podman (or Docker) — **experimental** per-task container isolation (`tsk task new … --container`)

## Install

| | Pacman | Cargo / from source |
|---|--------|---------------------|
| CLI | `/usr/bin/tsk` | `~/.cargo/bin/tsk` |
| Share templates + Waybar `.so` | `/usr/share/tsk/` | `~/.local/share/tsk/` via script |
| Runtime data | `~/.local/share/tsk/` | same |
| systemd | packaged unit | `scripts/install-systemd.sh` |

**Arch (pacman):**

```bash
cd packaging/arch && makepkg -si
systemctl --user enable --now tskd.service
```

Then source `/usr/share/tsk/hypr/bindings.conf` from Hyprland and merge the Waybar snippets under `/usr/share/tsk/waybar/`.

**Cargo / from source:**

```bash
cargo install --path crates/tsk-cli
scripts/install-user-share.sh
scripts/install-systemd.sh
```

Wire Hyprland/Waybar to `~/.local/share/tsk/` the same way.

**Omarchy** (auto-patches Hypr + Waybar after share assets exist):

```bash
tsk install omarchy
tsk doctor
```

Full steps, config examples, and uninstall: **[docs/install.md](docs/install.md)** · packaging paths: **[docs/packaging.md](docs/packaging.md)**

## Daily use

### Task manager

Open with **SUPER+Tab**, the Waybar task label, or `tsk task tui-launch`. Create, switch, and archive tasks there — or from the CLI:

```bash
tsk task new my-feature              # git/jj from cwd (or scratch if none)
tsk task new notes --scratch         # empty workspace under the task home
tsk task new fix --repo-path /path/to/checkout
tsk task new main --no-worktree      # use the main checkout (no worktree)
tsk task new iso --container         # experimental Distrobox isolation
tsk task list
tsk task switch my-feature
tsk task archive my-feature
tsk task restore my-feature
tsk task terminal                    # shell in the task checkout (Distrobox when --container)
tsk task editor                      # Cursor/VS Code (Distrobox when isolation is on)
tsk task browser                     # browser (Distrobox when isolation is on)
```

There is **experimental** support for container isolation with Distrobox: pass `--container` (or use the TUI checkbox) at create time. Terminals, editor, and browser then launch via `distrobox enter`. Image defaults live under `[distrobox]` in `~/.config/tsk/config.toml`.

Task homes live under `~/tsk-tasks/<id>/`. Linked checkouts are at `~/tsk-tasks/<id>/workspace/<repo-name>` (scratch tasks use the `workspace/` directory itself). Optional checkout settings live in `.tsk/repo.toml`.

On create/restore, tsk runs `.tsk/on-start.sh` (opens the editor via `tsk task editor` by default). See **[docs/cursor.md](docs/cursor.md)**.

```bash
tsk repo add                         # register cwd
tsk repo list
tsk repo root                        # detected git/jj root for cwd
```

### Keybindings (after Hyprland integration)

| Action | Binding |
|--------|---------|
| Task manager | **SUPER+Tab** (or Waybar task label) |
| Workspace 1–9 / 10 in current taskspace | **SUPER+1..9**, **SUPER+0** |
| Move window to workspace 1–10 | **SUPER+Shift+1..9 / 0** |
| Previous / next workspace | **SUPER+[** / **SUPER+]** (also trackpad swipe) |
| Default / host taskspace | **SUPER+H** or TUI → **host** |
| Task-aware terminal | **SUPER+Return** |
| Editor / browser | **SUPER+E** / **SUPER+B** |

Default and task taskspaces both use **10** slots so keybinds feel the same. Change the count with `workspace_count` under `[default]` in `~/.config/tsk/config.toml`.

### Useful commands

```bash
tsk doctor
tsk status
tsk taskspace default                # same as SUPER+H
tsk windows                          # list windows + task association
tsk windows restore                  # move windows back to home workspaces
tsk daemon status
```

## Troubleshooting

```bash
tsk doctor
tsk taskspace default
systemctl --user status tskd.service
hyprctl reload                       # after changing Hypr source lines
```

If the Waybar indicator is stuck, confirm `tskd.service` is running and the CFFI `module_path` points at the right `libtsk_waybar.so` (`/usr/share/tsk/lib/` or `~/.local/share/tsk/lib/`).

## More documentation

| Doc | For |
|-----|-----|
| [docs/install.md](docs/install.md) | Production install, manual Hypr/Waybar wiring |
| [docs/packaging.md](docs/packaging.md) | Arch package layout and AUR notes |
| [docs/dev.md](docs/dev.md) | Developing tsk itself (dev session, e2e) |
| [docs/cursor.md](docs/cursor.md) | Cursor / on-start hooks |
