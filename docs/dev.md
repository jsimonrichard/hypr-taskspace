# Development setup

Development uses a **separate install tree** for binaries and integration, but **shares prod task state** by default:

| | Prod | Dev |
|---|------|-----|
| Share dir | `~/.local/share/tsk` | `~/.local/share/tsk-dev` |
| Config | `~/.config/tsk/config.toml` | `~/.config/tsk-dev/config.toml` |
| CLI | `tsk` on PATH (pacman `/usr/bin/tsk`) | same entrypoint — prod `tsk` re-execs dev build when session file is present |
| Session file | — | `~/.local/share/tsk/dev-session` (one line: path to dev binary) |
| Daemon | systemd (`tskd.service`) | manual — `scripts/dev.sh daemon` |
| Hypr marker | `tsk-managed` | `tsk-dev-managed` |
| Task data | `~/tsk-tasks/` | same (`~/tsk-tasks/`) |
| Session DB | `~/.local/share/tsk/state.db` | symlink → prod `state.db` |
| Share templates | `/usr/share/tsk` (pacman) or `~/.local/share/tsk` (cargo) | `~/.local/share/tsk-dev/` |

## One-command dev mode

From the repo root:

```bash
scripts/dev.sh          # same as scripts/dev.sh enter
```

This:

1. Symlinks `~/.local/share/tsk-dev/state.db` → prod `state.db` (so existing tasks, repos, and window registry are visible)
2. Builds `target/release/tsk` and writes `~/.local/share/tsk/dev-session` with the dev binary path
3. Runs `tsk dev install all` (share assets, Hyprland, Waybar)
4. Stops prod `tskd.service` if running, starts the **dev daemon** in the foreground

When you exit the dev daemon (Ctrl+C) or run `scripts/dev.sh leave`, dev integration is **fully removed**: Hyprland and Waybar are restored to prod, the session file is deleted, and prod `tskd.service` is restarted if it was running before.

**No environment variables.** Hyprland keybinds, Waybar helpers, and new terminals all keep calling `tsk` (or `/usr/bin/tsk`). When the session file exists, prod `tsk` re-execs the dev build and loads dev config automatically — no Hyprland reload required for binary switching.

Set `TSK_DEV_ISOLATED=1` to use a separate dev `state.db` instead (CI/e2e).

When you first run dev, `~/.config/tsk-dev/config.toml` is created from your prod config (if present) with dev paths for the daemon socket and install tree. Settings like `global_workspaces` are copied as-is.

## Subcommands

```bash
scripts/dev.sh enter              # install all + start dev daemon
scripts/dev.sh leave              # uninstall dev integration + restore prod
scripts/dev.sh install all        # integration only (no daemon)
scripts/dev.sh install share        # build + swap PATH tsk + dev share assets
scripts/dev.sh daemon             # link prod state.db + start dev daemon
scripts/dev.sh status             # tsk dev status
scripts/dev.sh uninstall          # same as leave (integration only)
```

Equivalent CLI (with `TSK_WORKSPACE` set):

```bash
cargo run -p tsk-cli --release -- dev install all
cargo run -p tsk-cli --release -- dev install share
cargo run -p tsk-cli --release -- dev uninstall all
cargo run -p tsk-cli --release -- dev status
```

## Switching prod ↔ dev

Dev mode is **session-scoped**: entering installs dev Hyprland/Waybar integration and starts a foreground daemon; leaving (Ctrl+C or `scripts/dev.sh leave`) uninstalls dev integration and restores prod.

If a previous dev session ended uncleanly, `scripts/dev.sh enter` or `leave` detects stale integration (e.g. `tsk-dev-managed` still in `hyprland.conf`) and cleans up first.

Prod and dev Hypr bindings both call `tsk` on PATH (`/usr/bin/tsk` when installed via pacman). During an active dev session, the session file at `~/.local/share/tsk/dev-session` tells prod `tsk` to re-exec the repo build and use dev config — including for Hyprland `exec` bindings without restarting Hyprland.

**Note:** Dev mutates the shared prod `state.db` while the dev daemon runs (prod systemd is stopped, so there is only one writer). Avoid creating test tasks you do not want in prod, or use `TSK_DEV_ISOLATED=1`.

## E2E / CI

Use isolated state for automated tests:

```bash
TSK_DEV_ISOLATED=1 scripts/dev.sh install all
TSK_DEV_ISOLATED=1 scripts/dev.sh daemon &
```

Dry-run:

```bash
TSK_WORKSPACE=$PWD cargo run -p tsk-cli --release -- dev install all --dry-run
```
