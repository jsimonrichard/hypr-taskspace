# Hypr Taskspace

Task-centric Hyprland control plane. Each task gets its own **taskspace** with named **workspaces** (`auth-fix-1`, `auth-fix-2`, …). The **default** taskspace uses plain Hyprland workspace names **`1`–`10`** for everyday host work.

Hyprland keybinds call `tsk` on your **PATH**. Runtime state (`state.db`, daemon socket) lives in **`~/.local/share/tsk/`**. Templates and the Waybar module live under a **share tree** — either **`/usr/share/tsk/`** (pacman) or **`~/.local/share/tsk/`** (cargo/from source).

## Prerequisites

- Hyprland (Omarchy or similar)
- `hyprctl` on PATH
- **Rust toolchain** (stable) — only for building from source or development ([rustup](https://rustup.rs/))

Optional:

- `distrobox` + Podman (or Docker) — optional per-task container isolation (`tsk task new … --container`)

---

## Install

`tsk install` only automates **Omarchy** config patching (Hypr + Waybar with backups). Everything else — binaries, share files, systemd — comes from **packaging** or **repo scripts**.

| What | Pacman | Cargo / from source |
|------|--------|---------------------|
| CLI (`tsk`) | `/usr/bin/tsk` | `~/.cargo/bin/tsk` |
| Share templates + Waybar `.so` | `/usr/share/tsk/` | `~/.local/share/tsk/` via script |
| Runtime data | `~/.local/share/tsk/` | same |
| systemd unit | `/usr/lib/systemd/user/tskd.service` | script or copy from `share/systemd/` |
| Hypr/Waybar wiring | Manual (or `tsk install omarchy`) | Manual (or omarchy) |

Full details: **[docs/install.md](docs/install.md)** · Arch packaging: **[docs/packaging.md](docs/packaging.md)**

### 1. Arch Linux (pacman)

Best for a stable system install — no extra scripts for share assets.

```bash
cd packaging/arch && makepkg -si
systemctl --user enable --now tskd.service
```

Then wire Hyprland and Waybar yourself (paths under `/usr/share/tsk/`). Example Hypr line:

```ini
source = /usr/share/tsk/hypr/bindings.conf
```

Copy or merge snippets from `/usr/share/tsk/waybar/`. See **[docs/packaging.md](docs/packaging.md)**.

Suggested config (`~/.config/tsk/config.toml`):

```toml
[data]
dir = "~/.local/share/tsk"

[install.hypr]
share_dir = "/usr/share/tsk"
source_line = "/usr/share/tsk/hypr/bindings.conf"
```

Or copy `/usr/share/tsk/config.toml.example`.

### 2. Cargo / from source

For contributors or distros without a package yet.

```bash
cargo install --path crates/tsk-cli
scripts/install-user-share.sh    # templates + libtsk_waybar.so → ~/.local/share/tsk/
scripts/install-systemd.sh       # enable tskd.service
```

Then wire Hyprland and Waybar (paths under `~/.local/share/tsk/`). See **[docs/install.md](docs/install.md)**.

After pulling repo changes:

```bash
scripts/install-user-share.sh
```

### 3. Omarchy (automated Hypr + Waybar)

Patches `hyprland.conf` and Waybar config with backups — on top of pacman or script install.

```bash
# share assets must exist first (pacman or scripts/install-user-share.sh)
tsk install omarchy
scripts/install-systemd.sh
tsk doctor
```

Dry-run: `tsk install omarchy --dry-run`

### 4. Verify

```bash
tsk doctor
tsk integration status
systemctl --user status tskd.service
```

### Migrating / cleanup

If you previously copied everything into `~/.local/share/tsk/` and switch to pacman:

```bash
scripts/cleanup-legacy-install.sh    # removes duplicate templates; keeps state.db
```

---

## Development

Work on tsk from the repo without touching prod share assets or systemd (while the dev session runs).

| | Prod | Dev |
|---|------|-----|
| Share templates | `/usr/share/tsk` or `~/.local/share/tsk` | `~/.local/share/tsk-dev/` |
| Config | `~/.config/tsk/config.toml` | `~/.config/tsk-dev/config.toml` |
| CLI | `tsk` on PATH | release build **replaces** PATH `tsk` during dev session |
| Daemon | `tskd.service` (systemd) | foreground — `scripts/dev.sh daemon` |
| Session DB | `~/.local/share/tsk/state.db` | symlink → prod (default) |
| Task checkouts | `~/tsk-tasks/` | same |

### One-command dev session

From the repo root:

```bash
scripts/dev.sh          # same as scripts/dev.sh enter
```

This will:

1. Link dev `state.db` → prod (so existing tasks/windows stay visible)
2. Build `target/release/tsk` and swap it onto PATH (restored on exit)
3. Run `tsk dev install all` (dev share tree, Hyprland + Waybar integration)
4. Stop prod `tskd.service` if active, run the **dev daemon** in the foreground

Ctrl+C or `scripts/dev.sh leave` **fully disables dev mode**: uninstalls dev Hyprland/Waybar integration, restores the prod binary on PATH, and restarts prod systemd if it was running.

Use **`TSK_DEV_ISOLATED=1`** for a separate dev `state.db` (CI/e2e).

### Dev subcommands

```bash
scripts/dev.sh enter              # install all + start dev daemon
scripts/dev.sh leave              # uninstall dev integration + restore prod
scripts/dev.sh install all        # Hypr + Waybar integration only
scripts/dev.sh install share      # rebuild + swap PATH tsk + dev share assets
scripts/dev.sh daemon             # start dev daemon (links prod state.db)
scripts/dev.sh status             # tsk dev status
scripts/dev.sh uninstall            # same as leave (integration only)
```

Equivalent CLI (from repo, with `TSK_WORKSPACE=$PWD`):

```bash
cargo run -p tsk-cli --release -- dev install all
cargo run -p tsk-cli --release -- dev install share
cargo run -p tsk-cli --release -- dev uninstall all
```

Only one of prod or dev Hypr `source = … bindings.conf` lines should be active — comment the other out and `hyprctl reload` when switching.

See **[docs/dev.md](docs/dev.md)** for prod ↔ dev switching, e2e, and caveats about shared `state.db`.

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
tsk task new iso --container         # Distrobox isolation for terminals / editor / browser
tsk repo root                        # print detected git/jj root for cwd
tsk task list
tsk task switch my-feature
tsk task archive my-feature
tsk task terminal                    # task checkout shell (Distrobox enter when --container)
tsk task editor                      # Cursor/VS Code (inside Distrobox when isolation is on)
tsk task browser                     # Chromium/browser (inside Distrobox when isolation is on)
```

Enable Distrobox at create time with `--container` or the TUI checkbox. That runs `distrobox create --home ~/tsk-tasks/<id>` and routes **SUPER+Return** / **SUPER+E** / **SUPER+B** through `distrobox enter`. Tasks without isolation still launch host apps. Image defaults live in `[distrobox]` in `~/.config/tsk/config.toml`.

Task homes are created under `~/tsk-tasks/<id>/` for notes and agent metadata. Linked git/jj checkouts live under `~/tsk-tasks/<id>/workspace/<repo-folder-name>` (scratch tasks use `workspace/scratch`). Optional checkout settings (custom name, remote URL, VCS kind) live in `.tsk/repo.toml`; registered checkout paths and stable ids live in `state.db`.

Cursor opens on the task-specific checkout path (`TSK_TASK_REPO`). With container isolation, prefer `tsk task editor` (or the default on-start script) so Cursor runs via Distrobox. When multi-repo tasks are supported, tsk will open a task-level `.code-workspace` spanning all checkouts instead. See **[docs/cursor.md](docs/cursor.md)**.

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

### Keybindings (after Hyprland integration)

| Action | Binding |
|--------|---------|
| Task manager (TUI) | Click task name in Waybar, **SUPER+Tab**, or `tsk task tui-launch` |
| Workspace 1–9 / 10 within current taskspace | **SUPER+1..9**, **SUPER+0** (= workspace 10) — `tsk workspace switch` (hyprctl dispatch, then async state sync) |
| Default / host taskspace | **SUPER+H** or TUI → **host** |
| Task-aware terminal | **SUPER+Return** (override/remove in `bindings.conf` if you prefer your existing bind) |

SUPER+Space remains your normal system app launcher.

Default and task taskspaces both use **10** workspace slots (`1`–`10`, SUPER+0 → slot 10) so keybinds behave the same in either mode. Set `workspace_count = 10` under `[default]` in `~/.config/tsk/config.toml` to change the slot count for both.

### Waybar update path

State lives in `~/.local/share/tsk/state.db`. The daemon is the recommended single writer; the CLI falls back to direct DB access when the daemon is stopped. After every taskspace change:

1. Writes `state.db`
2. Bumps `$XDG_RUNTIME_DIR/tsk/state.rev`
3. Sends a JSON event on `$XDG_RUNTIME_DIR/tsk/state-events.sock` (Waybar listens here)
4. Signals Waybar (`RTMIN+11`) as a backup

The Waybar CFFI module subscribes to Hyprland workspace events **and** the state-events socket; `update()` also polls `state.rev` if both are missed.

```bash
systemctl --user status tskd.service   # prod
tsk doctor
```

### CLI reference

```text
tsk status | doctor | windows [--task ID]

tsk install omarchy              # Omarchy only: auto-patch Hypr + Waybar
tsk integration status
tsk daemon start|stop|restart|status|run

scripts/install-user-share.sh  # cargo/source: share templates + Waybar .so
scripts/install-systemd.sh     # enable tskd.service
scripts/dev.sh …               # development — see above

tsk dev install|uninstall|status   # dev integration (share, hypr, waybar)

tsk taskspace default|current
tsk workspace go|remember|dispatch|next|prev|goto

tsk task new|list|switch|current|archive|delete|menu|tui|tui-launch|terminal
tsk repo add|list|remove|root
tsk waybar status|module
tsk reset layout
tsk debug trace|hyprland-socket|hypr log
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
tsk task tui-launch
tsk doctor
```

Run `hyprctl reload` after adding the Hyprland source line so **SUPER+Tab** picks up the new bind.

**Waybar taskspace indicator stuck or laggy**

Check the CFFI `module_path` in Waybar config — pacman: `/usr/share/tsk/lib/libtsk_waybar.so`; cargo install: `~/.local/share/tsk/lib/libtsk_waybar.so`. Ensure `tskd.service` is running.

**Check overall health**

```bash
tsk doctor
tsk status
```
