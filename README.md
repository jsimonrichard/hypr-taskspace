# lae — Local Agentic Environment

Task-centric Hyprland control plane. Each task gets its own **taskspace** with named **workspaces** (`auth-fix-1`, `auth-fix-2`, …). The **default** taskspace uses plain Hyprland workspace names **`1`–`10`** for everyday host work.

The **Rust CLI** (`crates/lae-cli`) is the supported control plane. Hyprland keybinds call `~/.local/share/lae/bin/lae`, built and installed by `lae install hypr`. Run **`lae daemon start`** so one background process owns session state (recommended).

## Prerequisites

- Hyprland (Omarchy or similar)
- **Rust toolchain** (stable) — [rustup](https://rustup.rs/)
- `hyprctl` on PATH

Optional (not required for the Rust CLI):

- Python ≥ 3.11 — legacy Python package under `src/lae/` (daemon, distrobox, git clone); kept for future ports
- `distrobox` + Podman — deferred; `lae task terminal` is not in the Rust CLI yet

---

## Install

### 1. Build and install integrations

From the repo root:

```bash
cd ~/Desktop/local-agentic-env
LAE_WORKSPACE=$PWD cargo run -p lae-cli --release -- install all --dry-run   # optional preview
LAE_WORKSPACE=$PWD cargo run -p lae-cli --release -- install all
```

This builds the Rust `lae` binary and Waybar CFFI module, copies Hyprland templates to `~/.local/share/lae/`, patches Waybar config, and reloads Hyprland and Waybar.

After install, the CLI used by keybinds lives at:

```text
~/.local/share/lae/bin/lae
```

Add that directory to your shell PATH if you want to run `lae` outside Hyprland exec contexts. **`lae install hypr` also symlinks `~/.local/bin/lae` → the Rust binary** — put `~/.local/bin` early on PATH (before mise/venv shims):

```bash
export PATH="$HOME/.local/bin:$HOME/.local/share/lae/bin:$PATH"
```

If you previously ran `pip install -e .`, remove the old Python entry point:

```bash
pip uninstall local-agentic-env   # removes the pip `lae` script (now named lae-py if reinstalled)
```

Verify:

```bash
~/.local/share/lae/bin/lae --help
~/.local/share/lae/bin/lae status
lae doctor
lae daemon start    # recommended — single writer for state.db
lae daemon status
```

On first run, lae creates `~/.config/lae/config.toml` and `~/.local/share/lae/state.db`.

### 2. Check installation

```bash
lae doctor
lae install status
```

| Artifact | Location |
|----------|----------|
| Rust CLI + Waybar module | `~/.local/share/lae/bin/lae`, `~/.local/bin/lae` (symlink), `~/.local/share/lae/lib/liblae_waybar.so` |
| Hyprland keybinds + Omarchy unbinds | `~/.local/share/lae/hypr/bindings.conf`, `unbind-omarchy.conf` |
| Workspace keybind helper (hyprctl + state sync) | `~/.local/share/lae/bin/lae-workspace-switch` |
| Task manager launcher | `~/.local/share/lae/bin/lae-task-tui` |
| Config backup | `~/.local/share/lae/install/hypr/backups/<timestamp>/` |

Waybar uses a native **CFFI module** (`cffi/lae`) for instant taskspace/workspace indicators — no exec polling.

---

## Daily use

### Create and switch tasks

Use the **task manager TUI** (**SUPER+Tab**, click the task label in Waybar, or `lae task tui-launch`) to create, switch, and archive tasks interactively.

```bash
lae task tui                         # run TUI in the current terminal
lae task tui-launch                  # open TUI in your terminal emulator ([terminal].command)
lae task new my-feature              # uses git/jj repo from cwd (or scratch if none)
lae task new other --no-switch       # create without switching
lae task new notes --scratch         # isolated repo under ~/lae-tasks/<id>/repo
lae task new fix --repo-path /path/to/checkout
lae repo root                        # print detected git/jj root for cwd
lae task list
lae task switch my-feature
lae task archive my-feature
```

Task homes are created under `~/lae-tasks/<id>/` for notes and agent metadata. Repo settings live in each checkout at `.lae/repo.toml`; `~/.config/lae/repo-bookmarks.txt` only lists paths.

```bash
lae repo add                         # register cwd (writes .lae/repo.toml in the checkout)
lae repo list
lae repo remove <id>
lae repo root
```

### Switch back to the default taskspace

Open the **task manager** (Waybar task label, **SUPER+Tab**, or `lae task tui-launch`) and switch to **host**, or:

```bash
lae taskspace default        # SUPER+H
```

Legacy aliases still work: `lae context default`, `lae desktop go 1`, etc.

### Keybindings (after `lae install hypr`)

| Action | Binding |
|--------|---------|
| Task manager (TUI) | Click task name in Waybar, **SUPER+Tab**, or `lae task tui-launch` |
| Workspace 1–9 / 10 within current taskspace | **SUPER+1..9**, **SUPER+0** (= workspace 10) — `hyprctl dispatch` via `lae-workspace-switch`, then async state sync |
| Default / host taskspace | **SUPER+H** or TUI → **host** |
| Host terminal | **SUPER+Return** (your existing Omarchy bind — unchanged) |

SUPER+Space remains your normal system app launcher.

Default and task taskspaces both use **10** workspace slots (`1`–`10`, SUPER+0 → slot 10) so keybinds behave the same in either mode. Set `workspace_count = 10` under `[default]` in `~/.config/lae/config.toml` to change the slot count for both.

### Waybar update path

State lives in `~/.local/share/lae/state.db`. The **Rust daemon** (`lae daemon start`) is the recommended single writer; the CLI falls back to direct DB access when the daemon is stopped. After every taskspace change:

1. Writes `state.db`
2. Bumps `$XDG_RUNTIME_DIR/lae/state.rev`
3. Sends a JSON event on `$XDG_RUNTIME_DIR/lae/state-events.sock` (Waybar listens here)
4. Signals Waybar (`RTMIN+11`) as a backup

The Waybar CFFI module subscribes to Hyprland workspace events **and** the state-events socket; `update()` also polls `state.rev` if both are missed.

```bash
lae daemon start   # listens on ~/.local/share/lae/daemon.sock (see [daemon].socket in config)
lae doctor         # warns if daemon is not running
```

### CLI reference (Rust)

```text
lae status | doctor | windows [--task ID]

lae daemon start|stop|status|run
lae install all|hypr|waybar|status
lae uninstall hypr|waybar

lae taskspace default|current   # alias: context
lae workspace go|next|prev|goto                              # alias: desktop

lae task new|list|switch|current|archive|menu|tui|tui-launch
lae waybar refresh-cache|status|module
```

---

## Update after pulling changes

```bash
LAE_WORKSPACE=$PWD cargo run -p lae-cli --release -- install all
```

Re-run when `share/` templates, Rust code, or Waybar integration changes.

---

## Uninstall

Run in order. Integration uninstallers **restore your backed-up config files**; they do not delete task data unless you remove it manually.

```bash
lae uninstall waybar    # restores ~/.config/waybar/config.jsonc from backup
lae uninstall hypr      # restores ~/.config/hypr/hyprland.conf from backup
```

To keep lae-owned files under `~/.local/share/lae/hypr/` for inspection:

```bash
lae uninstall hypr --keep-files
```

Optional: remove task data and state (not done by uninstall):

```bash
rm -rf ~/.local/share/lae/state.db
rm -rf ~/lae-tasks/
rm -rf ~/.config/lae/
```

---

## Legacy Python package

The Python package in `src/lae/` is **not** required for daily use with the Rust CLI. It still contains:

- Legacy background **daemon** (superseded by `lae daemon start` in the Rust CLI)
- **`lae task terminal`** (Distrobox)
- **`lae task new --repo`** (git clone)
- Window routing on open/close events

To run the legacy Python CLI (development only):

```bash
pip install -e .
python -m lae.cli.main --help   # or: lae-py --help (after pip install)
```

The pip package no longer installs a `lae` command on PATH — use the Rust CLI from `lae install hypr` instead.

Do not mix pip-installed `lae-py` / old `lae` shims on PATH with `~/.local/share/lae/bin/lae` unless you know which one Hyprland is calling.

---

## Troubleshooting

**Wrong taskspace or stale state**

```bash
lae taskspace default
lae daemon start
sqlite3 ~/.local/share/lae/state.db "SELECT context_mode, current_task_id FROM session;"
```

**Task manager does not open**

```bash
~/.local/share/lae/bin/lae-task-tui   # should launch the TUI in a terminal
lae doctor
```

After `install hypr`, run `hyprctl reload` so **SUPER+Tab** picks up the new bind.

**Waybar taskspace indicator stuck or laggy**

```bash
LAE_WORKSPACE=$PWD cargo run -p lae-cli --release -- install waybar
```

**Check overall health**

```bash
lae doctor
lae status
```
