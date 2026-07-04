# Hypr Taskspace

Task-centric Hyprland control plane. Each task gets its own **taskspace** with named **workspaces** (`auth-fix-1`, `auth-fix-2`, …). The **default** taskspace uses plain Hyprland workspace names **`1`–`10`** for everyday host work.

Hyprland keybinds call `~/.local/share/tsk/bin/tsk`, built and installed by `tsk install hypr`. Run **`tsk daemon start`** so one background process owns session state (recommended).

## Prerequisites

- Hyprland (Omarchy or similar)
- **Rust toolchain** (stable) — [rustup](https://rustup.rs/)
- `hyprctl` on PATH

Optional:

- `distrobox` + Podman — deferred; container create/enter is not implemented yet (`tsk task terminal` opens a host shell in the task checkout)

---

## Install

### 1. Build and install integrations

From the repo root:

```bash
cd ~/Desktop/hypr-taskspace
TSK_WORKSPACE=$PWD cargo run -p tsk-cli --release -- install all --dry-run   # optional preview
TSK_WORKSPACE=$PWD cargo run -p tsk-cli --release -- install all
```

This builds the `tsk` binary and Waybar CFFI module, copies Hyprland templates to `~/.local/share/tsk/`, patches Waybar config, and reloads Hyprland and Waybar.

After install, the CLI used by keybinds lives at:

```text
~/.local/share/tsk/bin/tsk
```

Add that directory to your shell PATH if you want to run `tsk` outside Hyprland exec contexts. **`tsk install hypr` also symlinks `~/.local/bin/tsk` → the installed binary** — put `~/.local/bin` early on PATH:

```bash
export PATH="$HOME/.local/bin:$HOME/.local/share/tsk/bin:$PATH"
```

Verify:

```bash
~/.local/share/tsk/bin/tsk --help
~/.local/share/tsk/bin/tsk status
tsk doctor
tsk daemon start    # recommended — single writer for state.db
tsk daemon status
```

On first run, tsk creates `~/.config/tsk/config.toml` and `~/.local/share/tsk/state.db`.

### 2. Check installation

```bash
tsk doctor
tsk install status
```

| Artifact | Location |
|----------|----------|
| CLI + Waybar module | `~/.local/share/tsk/bin/tsk`, `~/.local/bin/tsk` (symlink), `~/.local/share/tsk/lib/libtsk_waybar.so` |
| Hyprland keybinds + Omarchy unbinds | `~/.local/share/tsk/hypr/bindings.conf`, `unbind-omarchy.conf` |
| Workspace keybind helper (hyprctl + state sync) | `~/.local/share/tsk/bin/tsk-workspace-switch` |
| Task manager launcher | `~/.local/share/tsk/bin/tsk-task-tui` |
| Config backup | `~/.local/share/tsk/install/hypr/backups/<timestamp>/` |

Waybar uses a native **CFFI module** (`cffi/tsk`) for instant taskspace/workspace indicators.

---

## Daily use

### Create and switch tasks

Use the **task manager TUI** (**SUPER+Tab**, click the task label in Waybar, or `tsk task tui-launch`) to create, switch, and archive tasks interactively.

```bash
tsk task tui                         # run TUI in the current terminal
tsk task tui-launch                  # open TUI in your terminal emulator ([terminal].command)
tsk task new my-feature              # uses git/jj repo from cwd (or scratch if none)
tsk task new other --no-switch       # create without switching
tsk task new notes --scratch         # isolated repo under ~/tsk-tasks/<id>/workspace/scratch
tsk task new fix --repo-path /path/to/checkout
tsk repo root                        # print detected git/jj root for cwd
tsk task list
tsk task switch my-feature
tsk task archive my-feature
tsk task terminal                    # host shell in current task checkout
```

Task homes are created under `~/tsk-tasks/<id>/` for notes and agent metadata. Linked git/jj checkouts live under `~/tsk-tasks/<id>/workspace/<repo-folder-name>` (scratch tasks use `workspace/scratch`). Repo settings live in each checkout at `.tsk/repo.toml`; `~/.config/tsk/repo-bookmarks.txt` only lists paths.

```bash
tsk repo add                         # register cwd (writes .tsk/repo.toml in the checkout)
tsk repo list
tsk repo remove <id>
tsk repo root
```

### Switch back to the default taskspace

Open the **task manager** (Waybar task label, **SUPER+Tab**, or `tsk task tui-launch`) and switch to **host**, or:

```bash
tsk taskspace default        # SUPER+H
```

### Keybindings (after `tsk install hypr`)

| Action | Binding |
|--------|---------|
| Task manager (TUI) | Click task name in Waybar, **SUPER+Tab**, or `tsk task tui-launch` |
| Workspace 1–9 / 10 within current taskspace | **SUPER+1..9**, **SUPER+0** (= workspace 10) — `hyprctl dispatch` via `tsk-workspace-switch`, then async state sync |
| Default / host taskspace | **SUPER+H** or TUI → **host** |
| Host terminal | **SUPER+Return** (your existing Omarchy bind — unchanged) |

SUPER+Space remains your normal system app launcher.

Default and task taskspaces both use **10** workspace slots (`1`–`10`, SUPER+0 → slot 10) so keybinds behave the same in either mode. Set `workspace_count = 10` under `[default]` in `~/.config/tsk/config.toml` to change the slot count for both.

### Waybar update path

State lives in `~/.local/share/tsk/state.db`. The daemon (`tsk daemon start`) is the recommended single writer; the CLI falls back to direct DB access when the daemon is stopped. After every taskspace change:

1. Writes `state.db`
2. Bumps `$XDG_RUNTIME_DIR/tsk/state.rev`
3. Sends a JSON event on `$XDG_RUNTIME_DIR/tsk/state-events.sock` (Waybar listens here)
4. Signals Waybar (`RTMIN+11`) as a backup

The Waybar CFFI module subscribes to Hyprland workspace events **and** the state-events socket; `update()` also polls `state.rev` if both are missed.

```bash
tsk daemon start   # listens on ~/.local/share/tsk/daemon.sock (see [daemon].socket in config)
tsk doctor         # warns if daemon is not running
```

### CLI reference

```text
tsk status | doctor | windows [--task ID]

tsk daemon start|stop|restart|status|run
tsk install all|hypr|waybar|status
tsk uninstall hypr|waybar

tsk taskspace default|current
tsk workspace go|remember|dispatch|next|prev|goto

tsk task new|list|switch|current|archive|delete|menu|tui|tui-launch|terminal
tsk repo add|list|remove|root
tsk waybar status|module
tsk reset layout
tsk debug trace|hyprland-socket|hypr log
```

---

## Update after pulling changes

```bash
TSK_WORKSPACE=$PWD cargo run -p tsk-cli --release -- install all
```

Re-run when `share/` templates, Rust code, or Waybar integration changes.

---

## Uninstall

Run in order. Integration uninstallers **restore your backed-up config files**; they do not delete task data unless you remove it manually.

```bash
tsk uninstall waybar    # restores ~/.config/waybar/config.jsonc from backup
tsk uninstall hypr      # restores ~/.config/hypr/hyprland.conf from backup
```

To keep tsk-owned files under `~/.local/share/tsk/hypr/` for inspection:

```bash
tsk uninstall hypr --keep-files
```

Optional: remove task data and state (not done by uninstall):

```bash
rm -rf ~/.local/share/tsk/state.db
rm -rf ~/tsk-tasks/
rm -rf ~/.config/tsk/
```

---

## Troubleshooting

**Wrong taskspace or stale state**

```bash
tsk taskspace default
tsk daemon start
sqlite3 ~/.local/share/tsk/state.db "SELECT context_mode, current_task_id FROM session;"
```

**Task manager does not open**

```bash
~/.local/share/tsk/bin/tsk-task-tui   # should launch the TUI in a terminal
tsk doctor
```

After `install hypr`, run `hyprctl reload` so **SUPER+Tab** picks up the new bind.

**Waybar taskspace indicator stuck or laggy**

```bash
TSK_WORKSPACE=$PWD cargo run -p tsk-cli --release -- install waybar
```

**Check overall health**

```bash
tsk doctor
tsk status
```
