# lae — Local Agentic Environment

Task-centric Hyprland control plane. Each task gets its own **taskspace** with named **workspaces** (`auth-fix-1`, `auth-fix-2`, …). The **default** taskspace uses plain Hyprland workspace names **`1`–`10`** for everyday host work.

The **Rust CLI** (`crates/lae-cli`) is the supported control plane. Hyprland keybinds and Walker menus call `~/.local/share/lae/bin/lae`, built and installed by `lae install hypr`.

## Prerequisites

- Hyprland (Omarchy or similar)
- **Rust toolchain** (stable) — [rustup](https://rustup.rs/)
- Walker + Elephant (Omarchy ships these)
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

This builds the Rust `lae` binary and Waybar CFFI module, copies Hyprland templates to `~/.local/share/lae/`, patches Waybar config, and reloads Hyprland, Walker, and Waybar.

After install, the CLI used by keybinds lives at:

```text
~/.local/share/lae/bin/lae
```

Add that directory to your shell PATH if you want to run `lae` outside Hyprland exec contexts:

```bash
export PATH="$HOME/.local/share/lae/bin:$PATH"
```

Verify:

```bash
~/.local/share/lae/bin/lae --help
~/.local/share/lae/bin/lae status
lae doctor
```

On first run, lae creates `~/.config/lae/config.toml` and `~/.local/share/lae/state.db`.

### 2. Check installation

```bash
lae doctor
lae install status
```

| Artifact | Location |
|----------|----------|
| Rust CLI + Waybar module | `~/.local/share/lae/bin/lae`, `~/.local/share/lae/lib/liblae_waybar.so` |
| Hyprland keybinds + Omarchy unbinds | `~/.local/share/lae/hypr/bindings.conf`, `unbind-omarchy.conf` |
| Walker menu helper | `~/.local/share/lae/bin/lae-task-menu-json` |
| Elephant menu | `~/.config/elephant/menus/lae_tasks.lua` |
| Config backup | `~/.local/share/lae/install/hypr/backups/<timestamp>/` |

Waybar uses a native **CFFI module** (`cffi/lae`) for instant taskspace/workspace indicators — no exec polling.

---

## Daily use

### Create and switch tasks

```bash
lae task new my-feature              # creates task dirs + Hyprland workspaces, switches in
lae task new other --no-switch       # create without switching
lae task list
lae task switch my-feature
lae task archive my-feature
```

Task homes are created under `~/lae-tasks/<id>/` (notes + empty `repo/` directory). Git clone and Distrobox setup are **not** part of the Rust CLI yet.

### Switch back to the default taskspace

Open the **task menu** (Waybar task label, SUPER+Tab, or `lae task menu`) and choose **default**, or:

```bash
lae taskspace default        # SUPER+H
```

Legacy aliases still work: `lae context default`, `lae desktop go 1`, etc.

### Keybindings (after `lae install hypr`)

| Action | Binding |
|--------|---------|
| Task menu (Walker) | Click task name in Waybar, **SUPER+Tab**, or `lae task menu` |
| Workspace 1–9 / 10 within current taskspace | **SUPER+1..9**, **SUPER+0** (= workspace 10) |
| Default / host taskspace | **SUPER+H** or Walker → **default** |
| Global escape hatch (all Hyprland workspaces) | **SUPER+Escape** |
| Host terminal | **SUPER+Return** (your existing Omarchy bind — unchanged) |

SUPER+Space remains the normal Walker app launcher, not the task menu.

Default taskspace supports **10** Hyprland workspaces (`1`–`10`). Waybar updates on taskspace/workspace changes via Hyprland socket events. Task taskspaces use 3 scoped workspaces by default. Set `workspace_count = 10` under `[default]` in `~/.config/lae/config.toml`.

### CLI reference (Rust)

```text
lae status | doctor | windows [--task ID]

lae install all|hypr|waybar|status
lae uninstall hypr|waybar

lae taskspace default|global|restore|toggle-global|current   # alias: context
lae workspace go|next|prev|goto                              # alias: desktop

lae task new|list|switch|current|archive|menu|menu-json
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

- Background **daemon** (UNIX socket IPC)
- **`lae task terminal`** (Distrobox)
- **`lae task new --repo`** (git clone)
- Window routing on open/close events

To run the legacy Python CLI (development only):

```bash
pip install -e .
python -m lae.cli.main --help
```

Do not mix pip-installed `lae` on PATH with `~/.local/share/lae/bin/lae` unless you know which one Hyprland is calling.

---

## Troubleshooting

**Walker task menu is empty**

Re-run install (reloads Walker automatically):

```bash
LAE_WORKSPACE=$PWD cargo run -p lae-cli --release -- install hypr
~/.local/share/lae/bin/lae-task-menu-json   # should list default + tasks
omarchy-restart-walker
systemctl --user restart elephant.service   # if still empty
```

**Waybar taskspace indicator stuck or laggy**

```bash
LAE_WORKSPACE=$PWD cargo run -p lae-cli --release -- install waybar
```

**Check overall health**

```bash
lae doctor
lae status
```
