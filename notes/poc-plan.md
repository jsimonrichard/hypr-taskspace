# Python POC Plan: Local Agentic Coding Environment Management

> **Implementation status (2026):** The **Rust CLI** in `crates/` is the supported control plane. See [README.md](../README.md). Hyprland keybinds use `~/.local/share/lae/bin/lae`; task management uses the **ratatui TUI** (`lae task tui-launch`). Waybar uses the Rust **CFFI module** (`cffi/lae`) with Hyprland socket2 for instant updates. The Python package in `src/lae/` remains for deferred features (daemon IPC, Distrobox terminals, git clone on `task new`, window routing). Command names: prefer **`taskspace`** / **`workspace`**; **`context`** / **`desktop`** are legacy aliases in the Rust CLI.

This plan expands on [ai-convo-notes.md](./ai-convo-notes.md) into an implementable proof-of-concept. The POC validates the core abstraction:

**task > workspace/desktop > window**

with Hyprland as the presentation/routing layer and Distrobox as the per-task execution environment containing an isolated clone of the target repository.

---

## POC Scope

### In scope

- Python control plane (CLI + optional background daemon)
- Task lifecycle: create, list, switch, archive
- One Distrobox container per task with a cloned repo inside
- Hyprland workspace routing via `hyprctl` and IPC event hooks
- **Desktop context isolation**: while in a task, only that task's desktops are reachable via normal navigation; a quick escape hatch restores access to all desktops
- **Default (system) desktops**: a fixed workspace group with no Distrobox association for normal host work
- Task-scoped terminal launch (host terminal → `distrobox enter`; default desktops launch on the host)
- Basic state persistence (JSON or SQLite on the host)
- Visible current-task indicator (CLI output + Waybar module stub)
- Window-to-task correlation via title prefix + workspace assignment

### Out of scope (for POC)

- Agent/LLM integration (stub hooks only)
- Browser profile automation (document pattern, defer implementation)
- Multi-machine sync / cloud resume
- Full session restore after reboot
- Editor-specific deep integration (Cursor/VS Code workspace files)
- GUI launcher (fuzzel/wofi palette is Phase 2+)

### Success criteria

1. User can run `lae task new auth-fix --repo https://github.com/org/app.git` and get a named task with a Distrobox container and repo clone.
2. User can run `lae task switch auth-fix` and Hyprland moves to that task's workspace group; new terminals open inside the task container.
3. While in `auth-fix`, workspace next/prev and `SUPER+1..3` only visit auth-fix's desktops—not other tasks or unrelated workspaces.
4. User can hit the escape hatch (`SUPER+ESCAPE` or `lae context global`) to navigate any desktop; returning to the task or default context re-scopes navigation.
5. User can run `lae context default` (or equivalent) to work on the system default desktops with no Distrobox; host terminals and apps launch normally.
6. User can run two tasks in parallel without mixing terminals, workspaces, or repo checkouts.
7. `lae status` always shows the current context (default / task / global), current task (if any), and windows/container/repo path.
8. `lae uninstall hypr` restores the user's Hyprland config from the pre-install backup; workspace keybinds behave as before lae was installed.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Host (Hyprland session)                                        │
│                                                                 │
│  ┌──────────────┐   hyprctl/IPC   ┌──────────────────────────┐ │
│  │ Waybar CFFI  │   socket2/IPC   │  lae CLI + lae-core (Rust) │ │
│  │ cffi/lae     │◄───────────────►│  - task registry (SQLite)  │ │
│  └──────────────┘                 │  - taskspace navigation    │ │
│                                   │  - hyprland event listener │ │
│  ┌──────────────┐   CLI          └───────────┬──────────────┘ │
│  │ lae CLI      │───────────────►              │
│  │ (Rust)       │                              │ (optional legacy)
│  └──────────────┘                              ▼
│                                   ┌──────────────────────────┐ │
│                                   │  lae daemon (Python)     │ │
│                                   │  - window router         │ │
│                                   │  - distrobox terminal    │ │
│                                   └──────────────────────────┘ │
│                                              │ distrobox/podman │
│  Hyprland workspaces (context-scoped nav)  ▼                  │
│  ┌──────────┬─────────┬─────────┐  ┌──────────────────────┐   │
│  │ default  │ task-A  │ task-B  │  │ Distrobox: lae-auth  │   │
│  │ ws 1-3   │ ws 10-12│ ws 13-15│  │  ~/tasks/auth-fix/   │   │
│  │ (no box) │         │         │  │    app/ (git clone)  │   │
│  └──────────┴─────────┴─────────┘  └──────────────────────┘   │
│         ▲  escape hatch → global context (all ws reachable)   │
│         ▲                           └──────────────────────┘   │
│         │ window rules / movetoworkspace                          │
│  ┌──────┴───────┐                                               │
│  │ kitty/alacritty│  title: [auth-fix] …                        │
│  └──────────────┘                                               │
└─────────────────────────────────────────────────────────────────┘
```

### Design principles

1. **Host owns routing; container owns execution.** Hyprland and the Python daemon run on the host. Code, packages, and tests run inside Distrobox.
2. **Task is the source of truth.** Workspaces, containers, clone paths, and window lists are all properties of a task record.
3. **Fail closed on ambiguity.** If context is `default`, new windows stay on default desktops. If context is a task, new windows belong to that task. Windows are never silently attached to the wrong task.
4. **Convention over magic.** Predictable naming (`lae-<task-id>`, `[task-id]` title prefix, workspace ranges) makes debugging easy.
5. **Scoped navigation by default.** Hyprland still has all workspaces internally, but user-facing navigation (keybinds, scroll, CLI) is filtered by the active **desktop context**. Isolation is enforced at the control plane, not by pretending other workspaces don't exist.

---

## Desktop Context & Isolation

Hyprland does not natively hide workspaces from the compositor. The POC achieves task isolation by owning **all workspace navigation** through the `lae` daemon instead of binding keys directly to `hyprctl dispatch workspace N`.

### Three desktop contexts

| Context | Meaning | Reachable workspaces | Distrobox |
|---------|---------|----------------------|-----------|
| `default` | Main system / home | Default group only (e.g. ws 1–3) | None |
| `task:<id>` | Inside a task environment | That task's group only (e.g. ws 10–12) | `lae-<id>` |
| `global` | Escape hatch | **All** workspaces (absolute navigation) | Depends on target |

The user should normally live in either `default` or `task:<id>`. `global` is a temporary overlay mode for when you need to peek at another task's desktop, drag a window, or debug routing.

### Escape hatch

**Trigger:** `SUPER+ESCAPE` (primary) or `lae context global` / `lae context escape`

**Behavior:**
1. Save previous context (`default` or `task:<id>`) in session state.
2. Set context → `global`.
3. Show a visible indicator (Waybar: `ALL DESKTOPS`, notify-send optional).
4. Rebind navigation to absolute workspace IDs (or enable a Hyprland submap for direct numeric jump—see below).
5. **Exit:** same keybind again, `lae context restore`, or `lae task switch` / `lae context default` → restore previous scoped context and jump back to the last focused workspace within it.

This prevents getting stuck behind task walls while keeping day-to-day navigation task-local.

### Default (system) desktops

The **default workspace group** always exists and is **not** tied to any task or Distrobox container:

- Used for email, system settings, non-task browsing, host IDE sessions, etc.
- `lae context default` switches to this context and focuses default workspace 1 (or last-used default ws).
- Terminals launched here use the host shell directly—no `distrobox enter`.
- New windows while in `default` context open on default workspaces only.

Entering a task (`lae task switch auth-fix`) leaves default desktops untouched in the background; they simply fall outside the navigation set until the user switches back or uses the escape hatch.

### How navigation scoping works

Replace stock Hyprland workspace binds with `lae`-mediated dispatchers:

```ini
# share/hypr/bindings.conf — all workspace movement goes through lae
bind = SUPER, 1, exec, lae desktop go 1
bind = SUPER, 2, exec, lae desktop go 2
bind = SUPER, 3, exec, lae desktop go 3
bind = SUPER, mouse_down, exec, lae desktop next
bind = SUPER, mouse_up, exec, lae desktop prev
bind = SUPER, bracketleft, exec, lae desktop prev
bind = SUPER, bracketright, exec, lae desktop next
bind = SUPER, ESCAPE, exec, lae context toggle-global
bind = SUPER, H, exec, lae context default
```

`lae desktop go N` resolves **relative** index `N` within the active context:

| Context | `desktop go 1` | `desktop go 2` | `desktop go 3` |
|---------|----------------|----------------|----------------|
| `default` | ws 1 (`default:main`) | ws 2 | ws 3 |
| `task:auth-fix` | ws 10 (`auth-fix:main`) | ws 11 | ws 12 |
| `global` | ws 1 (absolute) | ws 2 | ws 3 |

In `global` context, `lae desktop go` uses absolute IDs. For workspaces beyond the default key row, provide a submap or `lae desktop goto <absolute>` (Phase 2).

`lae desktop next/prev` cycles only within the current context's workspace list—never wrapping into another task's range.

### Optional: Hyprland submap for global jump

For the escape hatch, an alternative or supplement to relative binds is a **global workspace submap** (similar to Omarchy-style workspace pickers):

```ini
bind = SUPER ALT, G, exec, lae context global
submap = lae-global
bind = , 1, exec, lae desktop goto 1
bind = , 2, exec, lae desktop goto 2
# ... digits 0-9 for quick absolute jump ...
bind = , ESCAPE, exec, lae context restore
submap = reset
```

The daemon enters this submap via `hyprctl dispatch submap lae-global` when global context activates, ensuring bare digit keys only fire while escaping.

---

## Workspace Layout Strategy (Hyprland)

Two fixed zones: **default (system)** and **task slots**.

### Default zone (always present)

| Workspaces | Purpose (convention) |
|------------|----------------------|
| 1 | main (general host work) |
| 2 | comms / browser |
| 3 | aux |

No Distrobox. Names via `hyprctl keyword workspace`:

```
1:name:default:main
2:name:default:comms
3:name:default:aux
```

### Task zones (allocated on `task new`)

Each task gets a **contiguous workspace range** starting after the default zone:

| Slot | Workspaces | Example task | Purpose |
|------|------------|--------------|---------|
| — | 1–3 | *(default)* | system desktops |
| 0 | 10–12 | auth-fix | main / browser / aux |
| 1 | 13–15 | billing | main / browser / aux |
| 2 | 16–18 | … | … |

Workspaces 4–9 are reserved as a buffer (unused in POC) so default and task groups don't abut—makes debugging and absolute jump easier.

Task workspace names:

```
10:name:auth-fix:main
11:name:auth-fix:browser
12:name:auth-fix:aux
```

POC uses fixed slot assignment (first free slot). The **desktop context** determines which of these ranges is navigable—not which exist in Hyprland.

### hyprctl operations the daemon will use

| Operation | Command / API | When |
|-----------|---------------|------|
| Switch workspace | `hyprctl dispatch workspace <id>` | `task switch` |
| Move window | `hyprctl dispatch movetoworkspace <id>` | window tagged with task |
| Rename workspace | `hyprctl keyword workspace <id>,name:<name>` | task create |
| Query windows | `hyprctl clients -j` | reconcile window map |
| Query active workspace | `hyprctl activeworkspace -j` | status / switch detection |
| Subscribe to events | `hyprctl socket -j` or `socat` on `$HYPRLAND_INSTANCE_SIGNATURE` | auto-tag new windows |

---

## Distrobox + Repo Model

### Host directory layout

```
~/.local/share/lae/           # XDG data: shipped static configs, state DB, install backups
  hypr/
    bindings.conf             # lae keybinds (copied from repo on install)
  waybar/
    lae-context.jsonc         # optional module defs (copied from repo on install)
  install/
    hypr/manifest.json
    hypr/backups/<timestamp>/ # full copies of user files before any edit
~/.config/lae/config.toml     # user preferences (workspace slot size, base image, etc.)
~/lae-tasks/                  # task working trees (bind-mounted into containers)
  auth-fix/
    repo/                     # git clone lives here
  billing-refactor/
    repo/
```

### Container naming

- Container: `lae-<task_id>` (e.g. `lae-auth-fix`)
- Create: `distrobox create --name lae-auth-fix --image <config.image> --home ~/lae-tasks/auth-fix`
- Enter: `distrobox enter lae-auth-fix`

Using `--home` per task gives each container a distinct home directory while keeping clones on the host filesystem at a predictable path (`~/lae-tasks/<task_id>/repo`).

### Repo clone workflow

On `task new`:

1. Allocate task ID (slug from name, dedupe if collision).
2. Create host dir `~/lae-tasks/<task_id>/repo`.
3. `git clone <url> ~/lae-tasks/<task_id>/repo` (or init empty repo if no URL).
4. Create Distrobox container with home/bind mount covering the task directory.
5. Optionally run post-create hook inside container (`pip install -e .`, etc.)—POC: skip or manual.

On `task terminal`:

```bash
distrobox enter lae-<task_id> -- bash -lc 'cd ~/repo && exec $SHELL'
```

Host terminal emulator is launched with:
- `--working-directory ~/lae-tasks/<task_id>/repo` (for host-side tools if needed)
- `--title "[<task_id>] terminal"` (for window correlation)
- command wrapper that execs distrobox enter

---

## Python Project Structure

```
local-agentic-env/
├── pyproject.toml
├── src/lae/
│   ├── __init__.py
│   ├── cli/                    # Typer entry points
│   │   ├── __init__.py
│   │   ├── main.py             # `lae` root group
│   │   ├── task.py             # task subcommands
│   │   └── daemon.py           # daemon subcommands
│   ├── core/
│   │   ├── models.py           # Task, Window, TaskStatus (Pydantic)
│   │   ├── registry.py         # CRUD + current task
│   │   └── config.py           # load ~/.config/lae/config.toml
│   ├── integrations/
│   │   ├── hyprland.py         # hyprctl wrapper + JSON parsing
│   │   ├── hyprland_events.py  # socket listener
│   │   ├── distrobox.py        # create/enter/list containers
│   │   └── git.py              # clone/init repo on host
│   ├── install/
│   │   ├── manifest.py         # record/replay install changes
│   │   ├── backup.py           # timestamped config backups
│   │   └── hypr.py             # install / uninstall / status
│   ├── daemon/
│   │   ├── server.py           # UNIX socket IPC
│   │   ├── service.py          # orchestration logic
│   │   ├── desktop_nav.py      # context-scoped workspace go/next/prev
│   │   └── window_router.py    # match events → move/tag windows
│   └── util/
│       ├── subprocess.py       # safe run, capture JSON
│       └── xdg.py                # paths for config/data
├── share/                      # static templates → copied to ~/.local/share/lae/ on install
│   ├── hypr/
│   │   ├── bindings.conf       # lae keybinds (sourced by user's hyprland.conf)
│   │   └── window-rules.conf   # optional static rules
│   └── waybar/
│       └── lae-context.jsonc   # module defs + install instructions for modules-right
└── notes/
```

**Config ownership model (Omarchy-style):**

| Location | Role | Edited by |
|----------|------|-----------|
| `share/` in repo | Canonical static templates | lae developers |
| `~/.local/share/lae/` | Installed copies of templates | `lae install` only (overwrite on upgrade) |
| `~/.config/hypr/hyprland.conf` etc. | User's live config | User — lae only inserts marked `source` lines |
| `~/.local/share/lae/install/backups/` | Full pre-edit copies of user files | lae install (never auto-deleted) |

Shipped templates live in the repo under `share/`. **`lae install`** copies them to `~/.local/share/lae/`, inserts minimal `source` hooks into the user's existing config entrypoints, and backs up **entire** user config files before any edit so **`lae uninstall`** can restore byte-for-byte without manual surgery.

### Dependencies (pyproject.toml)

| Package | Purpose |
|---------|---------|
| `typer` | CLI |
| `pydantic` | models + validation |
| `tomli` / `tomli-w` | config |
| `httpx` or stdlib | optional, prefer subprocess for hyprctl |
| `watchdog` or asyncio socket | hyprland event loop |

Keep dependencies minimal for POC.

### CLI surface (Rust — current)

```bash
# Install (builds lae + Waybar CFFI, patches configs)
LAE_WORKSPACE=$PWD cargo run -p lae-cli --release -- install all|hypr|waybar|status
lae uninstall hypr|waybar
lae doctor                         # verify bindings, task TUI launcher, Waybar CFFI, SUPER+1

# Taskspace (alias: context)
lae taskspace default|global|restore|toggle-global|current

# Workspace navigation (alias: desktop) — Hyprland keybinds call these
lae workspace go <1-10>|next|prev|goto <name>

# Tasks (Rust: no --repo / no terminal yet)
lae task new <name> [--no-switch]
lae task list [--json]
lae task switch|current|archive|menu|tui|tui-launch

lae status
lae windows [--task <id>]
lae waybar refresh-cache|status|module
```

Legacy Python-only (see `src/lae/`): `lae daemon *`, `lae task new --repo`, `lae task terminal`.

### CLI surface (Python POC — original plan)

```bash
lae daemon start|stop|status     # background control plane

# Context (which desktop set is navigable) — use taskspace in Rust
lae context default              # switch to system default desktops (no task)
lae context global               # escape hatch: all desktops
lae context restore              # exit global, return to saved context
lae context toggle-global        # bound to SUPER+ESCAPE
lae context current              # print: default | task:auth-fix | global

# Desktop navigation (used by Hyprland keybinds)
lae desktop go <1-3>             # relative workspace within current context
lae desktop next|prev            # cycle within current context only
lae desktop goto <absolute>      # absolute ws id (global context / submap)

# Tasks
lae task new <name> [--repo URL] [--branch BR]
lae task list
lae task switch <name|id>        # sets context → task:<id>, scopes nav
lae task current
lae task terminal [<name>]       # default: current task; errors in default context
lae task terminal --host [<name>] # force host shell (rare)
lae task archive <name>

lae status                         # context + task + hypr + container summary
lae windows [--task <name>]        # list correlated windows

# Hyprland integration (reversible install)
lae install hypr [--dry-run]       # integrate lae keybinds into Hyprland config
lae uninstall hypr [--keep-files]  # restore pre-install Hyprland state
lae install status                 # show what lae has installed / modified
lae doctor                         # verify lae + hypr binds + daemon health
```

All mutating commands talk to the daemon if running; otherwise fall back to direct execution (single-process mode for debugging).

**Note:** `lae task terminal` without a current task context should error with a hint to `lae task switch` or use a plain host terminal via default context.

---

## Data Model

```python
class ContextMode(str, Enum):
    default = "default"              # system desktops, no task
    task = "task"                    # scoped to current_task_id
    global_ = "global"               # escape hatch: all desktops

class TaskStatus(str, Enum):
    active = "active"
    idle = "idle"
    archived = "archived"

class Task(BaseModel):
    id: str                          # slug, e.g. "auth-fix"
    name: str                        # display name
    status: TaskStatus
    repo_url: str | None
    repo_path: Path                  # host path to clone
    branch: str | None
    container_name: str              # lae-auth-fix
    workspace_range: tuple[int, int] # (10, 12) — absolute Hyprland ids
    workspaces: dict[str, int]       # {"main": 10, "browser": 11, "aux": 12}
    browser_profile: str | None      # POC: optional path, Phase 2
    created_at: datetime
    last_active_at: datetime
    agent_notes_path: Path | None    # stub for future agent state
    ports: list[int]                 # stub for dev servers

class WindowRecord(BaseModel):
    hypr_address: str              # Hyprland window address (hex)
    task_id: str | None            # None = default/system window
    title: str
    class_: str                      # app class
    workspace: int
    pid: int | None

class SessionState(BaseModel):
    context_mode: ContextMode
    current_task_id: str | None    # set when context_mode == task
    previous_context: ContextMode | None   # saved when entering global
    previous_task_id: str | None           # saved when entering global
    last_workspace: dict[str, int]           # per-context last ws, e.g. {"default": 1, "task:auth-fix": 10}
    default_workspace_range: tuple[int, int] # (1, 3) from config
    tasks: dict[str, Task]
    windows: dict[str, WindowRecord]  # keyed by hypr address
```

Persistence: SQLite via stdlib `sqlite3` with JSON columns, or a single `state.json` for POC simplicity. Recommend SQLite early—it avoids rewrite pain when window lists grow.

---

## Component Details

### 1. Hyprland integration (`integrations/hyprland.py`)

Wrapper around `hyprctl -j`:

```python
def hyprctl_json(*args: str) -> Any: ...
def get_clients() -> list[HydrWindow]: ...
def get_active_workspace() -> Workspace: ...
def dispatch(*cmd: str) -> None: ...
def set_workspace_name(id: int, name: str) -> None: ...
def move_window_to_workspace(address: str, workspace: int) -> None: ...
```

Parse stable fields from `hyprctl clients -j`: `address`, `title`, `class`, `workspace`, `pid`.

### 2. Event listener (`integrations/hyprland_events.py`)

Connect to Hyprland event socket (`$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket2.sock`).

Handle events:

- `openwindow` → if title matches `\[<task_id>\]`, assign to that task; else if context is `default`, leave on default workspaces; if context is `task`, move untagged windows to current task's main ws
- `closewindow` → remove from window registry
- `activewindow` → optional: in `global` context, highlight which task zone the focused window belongs to (status only; no auto-switch by default)
- `workspace` → update last_workspace for current context key (`default`, `task:<id>`, or `global`)

POC routing rule (simple):

> When a new window opens and its title matches `\[<task_id>\]`, assign it to that task's `main` workspace (or infer workspace from launch metadata).

### 3. Distrobox integration (`integrations/distrobox.py`)

```python
def container_exists(name: str) -> bool: ...
def create_container(name: str, home: Path, image: str) -> None: ...
def enter_command(name: str, cmd: str) -> list[str]: ...  # argv for host terminal
def is_running(name: str) -> bool: ...  # optional: distrobox list
```

Use `distrobox create -Y` for non-interactive POC.

### 4. Desktop navigator (`daemon/desktop_nav.py`)

Core isolation logic:

```python
def allowed_workspaces(state: SessionState) -> list[int]: ...
def desktop_go(state: SessionState, relative: int) -> None: ...   # hyprctl dispatch
def desktop_next(state: SessionState) -> None: ...
def desktop_prev(state: SessionState) -> None: ...
def desktop_goto_absolute(state: SessionState, ws: int) -> None: ...  # global only
def set_context(state: SessionState, mode: ContextMode, task_id: str | None) -> None: ...
def toggle_global(state: SessionState) -> None: ...
```

When `context_mode == task`, `allowed_workspaces()` returns only that task's range. Attempts to `desktop_go` outside the range are no-ops (or clamp with a notify-send in debug mode).

### 5. Task service (`daemon/service.py`)

Orchestrates:

- **create_task**: dirs → git clone → distrobox create → allocate workspaces → register
- **switch_task**: set context → `task:<id>` → `hyprctl dispatch workspace <main_ws>` → update Waybar/context files
- **context_default**: set context → `default` → focus last default workspace
- **context_global / restore**: escape hatch toggle with saved previous context
- **open_terminal**: in task context → distrobox enter; in default context → host shell
- **reconcile_windows**: periodic poll `hyprctl clients -j` vs registry

### 6. Daemon IPC (`daemon/server.py`)

UNIX socket at `$XDG_RUNTIME_DIR/lae/daemon.sock`.

JSON-line protocol (POC):

```json
{"method": "switch_task", "params": {"task_id": "auth-fix"}}
{"method": "set_context", "params": {"mode": "default"}}
{"method": "toggle_global", "params": {}}
{"method": "desktop_go", "params": {"relative": 1}}
{"method": "get_state"}
```

Enables fast CLI without reloading Hyprland state each invocation.

---

## User Flows

### Flow A: Create and enter a task

```
User: lae task new auth-fix --repo git@github.com:org/app.git

  1. Registry allocates slot workspaces 10-12
  2. git clone → ~/lae-tasks/auth-fix/repo
  3. distrobox create --name lae-auth-fix --home ~/lae-tasks/auth-fix
  4. hyprctl keyword workspace 10,name:auth-fix:main (×3)
  5. Registry saves task; context → task:auth-fix
  6. hyprctl dispatch workspace 10

User: lae task terminal

  7. Launch: kitty --title "[auth-fix] terminal" -- distrobox enter lae-auth-fix ...
  8. Event listener registers window → task auth-fix, workspace 10

User: SUPER+2

  9. lae desktop go 2 → ws 11 only (still within auth-fix; cannot reach default or billing)
```

### Flow B: Parallel tasks with scoped navigation

```
User: lae task new billing --repo ...
  → workspaces 13-15

User: lae task switch auth-fix
  → context task:auth-fix, workspace 10
  → SUPER+1..3 and scroll only visit ws 10-12

User: lae task switch billing
  → context task:billing, workspace 13
  → nav scoped to ws 13-15 only

Terminals launched in each context only enter their container.
Window titles disambiguate: [auth-fix] vs [billing].
```

### Flow C: Default (system) desktops

```
User: lae context default   # or SUPER+H

  1. context → default, focus last default ws (e.g. ws 1)
  2. SUPER+1..3 visits ws 1-3 only
  3. lae task terminal → error: "no task context; use lae task switch"
  4. Host terminal (SUPER+Return or similar) opens normal shell—no distrobox

User: lae task switch auth-fix
  → leaves default desktops in place, but nav no longer reaches them until restore/default/global
```

### Flow D: Escape hatch

```
User: (in task auth-fix, ws 10) presses SUPER+ESCAPE

  1. previous_context saved: task:auth-fix @ ws 10
  2. context → global; Waybar shows "ALL DESKTOPS"
  3. SUPER+1..9 or submap can jump to any absolute workspace (e.g. ws 14 to peek at billing)
  4. User presses SUPER+ESCAPE again (or lae context restore)
  5. context → task:auth-fix, hyprctl dispatch workspace 10
```

### Flow E: Status check

```
User: lae status

Context: task:auth-fix (scoped: ws 10-12)
Escape: SUPER+ESCAPE for global
Container: lae-auth-fix (running)
Repo: ~/lae-tasks/auth-fix/repo @ feature/oauth-fix
Windows:
  [auth-fix] terminal  → ws 10
  [auth-fix] nvim       → ws 10
Other tasks: billing (idle, ws 13-15, not reachable until switch or global)
Default desktops: ws 1-3 (reachable via lae context default or global)
```

---

## Hyprland Config Integration (Reversible Install)

Integrating lae with Hyprland must be **fully reversible** and **minimally invasive**—similar to how Omarchy sources defaults from `~/.local/share/omarchy/` while keeping user overrides in `~/.config/hypr/`.

Lae owns the static integration files; the user's config stays theirs except for one (or few) marked `source` lines the installer inserts.

### Design rules

1. **Static files live in the repo** under `share/`; install copies them to `~/.local/share/lae/`.
2. **Never write lae keybinds into `~/.config/hypr/`** — user config files are not lae's home.
3. **Only insert `source` lines** into user config entrypoints (e.g. `hyprland.conf`, `config.jsonc`); do not rewrite, merge, or comment inside the user's bindings/looknfeel/monitors files unless absolutely necessary and always with full-file backup first.
4. **Backup entire files before editing** — if `hyprland.conf` gets a source line appended, copy the whole file to `install/backups/<timestamp>/hyprland.conf` first. Uninstall restores the full file, not a reconstructed patch.
5. **Manifest every host-file mutation** — which files were backed up, which source lines were inserted, line numbers.
6. **Uninstall restores backups** — user config returns to exact pre-install bytes.
7. **Upgrade refreshes only `~/.local/share/lae/`** — updating lae overwrites shipped templates without touching user config (unless a migration adds a new required source line, recorded in manifest).

### Directory layout after install

```
~/.local/share/lae/                    # lae-owned (safe to delete on uninstall)
├── hypr/
│   ├── bindings.conf                  # from repo share/hypr/bindings.conf
│   └── window-rules.conf              # optional
├── waybar/
│   └── lae-context.jsonc              # module definitions
└── install/
    └── hypr/
        ├── manifest.json
        └── backups/
            └── 2026-06-27T143000/
                └── hyprland.conf      # FULL file before lae touched it

~/.config/hypr/
└── hyprland.conf                      # user's file — ONE lae line added at end:
                                       # source = ~/.local/share/lae/hypr/bindings.conf  # lae-managed
```

User files like `~/.config/hypr/bindings.conf`, `monitors.conf`, `looknfeel.conf` are **never modified** by lae.

### What gets installed

| Artifact | Source (repo) | Installed to | User config change |
|----------|---------------|--------------|-------------------|
| Hyprland keybinds | `share/hypr/bindings.conf` | `~/.local/share/lae/hypr/bindings.conf` | one `source` line in `hyprland.conf` |
| Window rules (optional) | `share/hypr/window-rules.conf` | `~/.local/share/lae/hypr/window-rules.conf` | one `source` line (or chained from bindings.conf) |
| Waybar module (optional) | `share/waybar/lae-context.jsonc` | `~/.local/share/lae/waybar/lae-context.jsonc` | documented manual step or JSONC patch with backup |
| Install manifest | — | `~/.local/share/lae/install/hypr/manifest.json` | none |
| Config backups | — | `~/.local/share/lae/install/hypr/backups/<ts>/` | none |

### Omarchy compatibility

Omarchy's `~/.config/hypr/hyprland.conf` ends with:

```ini
# Add any other personal Hyprland configuration below
```

The lae installer appends its source line **after** existing user sources—typically as the last line of `hyprland.conf`:

```ini
source = ~/.local/share/lae/hypr/bindings.conf  # lae-managed (installed 2026-06-27)
```

Because Hyprland uses the **last** matching bind for a key combo, lae's bindings (sourced last) override Omarchy's `~/.config/hypr/bindings.conf` workspace keys without editing Omarchy's files. Uninstall removes that one line (via full-file restore) and Omarchy behaves exactly as before.

### Install flow (`lae install hypr`)

```
1. Detect Hyprland config entrypoint
   - ~/.config/hypr/hyprland.conf (default)
   - override via config.toml [install.hypr] config_path

2. If already installed → refresh ~/.local/share/lae/hypr/ from repo templates;
   skip user config edits unless manifest says source line is missing

3. Copy static templates (always)
   share/hypr/* → ~/.local/share/lae/hypr/

4. Create full-file backups for any user config we will edit
   - cp hyprland.conf → install/hypr/backups/<iso-timestamp>/hyprland.conf
   - same for waybar config.jsonc if installing waybar integration

5. Insert source hook(s) — minimal edit only
   - Append to hyprland.conf (if absent):
     source = ~/.local/share/lae/hypr/bindings.conf  # lae-managed (installed 2026-06-27)
   - Optionally also source window-rules.conf from bindings.conf internally

   Do NOT edit bindings.conf, monitors.conf, or any other user-owned file.

6. Handle edge-case conflicts (rare)
   - If something sources AFTER hyprland.conf's lae line (unusual), lae doctor warns
   - Last resort only: comment a conflicting line in a user file — but ONLY after
     backing up that entire file and recording the exact original in manifest.json

7. Write manifest.json

8. Print summary + hyprctl reload
```

`--dry-run` prints planned copies, backup paths, and the exact source line(s) to insert.

### Uninstall flow (`lae uninstall hypr`)

```
1. Read manifest.json

2. Restore full backed-up user config files
   - install/hypr/backups/<install-timestamp>/hyprland.conf → ~/.config/hypr/hyprland.conf
   - restore any other backed-up files (waybar, etc.)

3. Remove lae-owned share tree (unless --keep-files)
   - rm -rf ~/.local/share/lae/hypr/
   - (preserve ~/.local/share/lae/install/backups/ and task state DB)

4. Verify: grep for lae-managed source lines — report stragglers if backup restore missed one

5. Clear install/hypr/manifest.json (backups retained)

6. Print: "Restored hyprland.conf from backup <timestamp>. Run hyprctl reload."
```

**`--keep-files`**: restore user config from backup but leave `~/.local/share/lae/hypr/` for inspection.

### Manual rollback (lae broken or uninstalled)

1. Restore `~/.config/hypr/hyprland.conf` from `~/.local/share/lae/install/hypr/backups/<latest>/hyprland.conf`
2. Optionally remove `~/.local/share/lae/hypr/`
3. `hyprctl reload`

No line-by-line editing required—the backup is the complete original file.

### Install manifest schema

```json
{
  "version": 1,
  "integration": "hypr",
  "installed_at": "2026-06-27T14:30:00-04:00",
  "backup_dir": "~/.local/share/lae/install/hypr/backups/2026-06-27T143000",
  "templates_installed": [
    {"from": "share/hypr/bindings.conf", "to": "~/.local/share/lae/hypr/bindings.conf"},
    {"from": "share/hypr/window-rules.conf", "to": "~/.local/share/lae/hypr/window-rules.conf"}
  ],
  "user_files_backed_up": [
    {"path": "~/.config/hypr/hyprland.conf", "backup": "hyprland.conf"}
  ],
  "user_files_modified": [
    {
      "path": "~/.config/hypr/hyprland.conf",
      "actions": [
        {
          "type": "append",
          "line": "source = ~/.local/share/lae/hypr/bindings.conf  # lae-managed (installed 2026-06-27)"
        }
      ]
    }
  ]
}
```

### Repo template (`share/hypr/bindings.conf`)

Canonical copy in repo; installed to `~/.local/share/lae/hypr/bindings.conf`:

```ini
# LAE task-environment keybinds — installed to ~/.local/share/lae/hypr/
# Do not edit here; customize via lae config or fork share/hypr/ in the repo.
# User's hyprland.conf sources this file. Remove via: lae uninstall hypr

source = ~/.local/share/lae/hypr/window-rules.conf

# Context switching
bind = SUPER, H, exec, lae context default
bind = SUPER, ESCAPE, exec, lae context toggle-global

# Scoped desktop navigation (relative within current context)
bind = SUPER, 1, exec, lae desktop go 1
bind = SUPER, 2, exec, lae desktop go 2
bind = SUPER, 3, exec, lae desktop go 3
bind = SUPER, bracketleft, exec, lae desktop prev
bind = SUPER, bracketright, exec, lae desktop next
bind = SUPER, mouse_down, exec, lae desktop next
bind = SUPER, mouse_up, exec, lae desktop prev

# Task actions
bind = SUPER, T, exec, lae task terminal
bind = SUPER, Tab, exec, lae task switch --interactive  # Phase 2
bind = SUPER SHIFT, N, exec, lae task new --prompt     # Phase 2

bind = SUPER ALT, G, exec, lae context global
```

Dynamic rules via `hyprctl keyword windowrule` are avoided in POC—static rules go in `window-rules.conf` under the same share tree.

### `lae doctor` checks

- [ ] `~/.local/share/lae/hypr/bindings.conf` exists and matches installed version
- [ ] `hyprland.conf` contains lae-managed source line
- [ ] lae source is last among top-level sources in entrypoint (best-effort parse)
- [ ] `SUPER+1` resolves to `lae desktop go 1` (`hyprctl binds -j`)
- [ ] backup for current install exists and is readable
- [ ] daemon reachable (if expected)

### Optional Waybar install (`lae install waybar`)

Same pattern:

1. Copy `share/waybar/lae-context.jsonc` → `~/.local/share/lae/waybar/lae-context.jsonc`
2. **Full backup** of `~/.config/waybar/config.jsonc` (or user's waybar entrypoint)
3. Minimal edit: add `"custom/lae-context"` to `modules-right` and merge module defs—or document a one-line include if Waybar supports it on the user's version
4. Separate manifest under `install/waybar/`
5. `lae uninstall waybar` restores waybar config from full backup

Waybar is optional and independent of Hyprland install/uninstall.

### Config reference (`config.toml`)

```toml
[install.hypr]
config_path = "~/.config/hypr/hyprland.conf"   # user entrypoint — only file lae edits
share_dir = "~/.local/share/lae"               # where templates are copied
source_line = "~/.local/share/lae/hypr/bindings.conf"
require_sourced_last = true                    # warn in doctor if violated
allow_user_file_comments = false               # never comment inside user files by default
```

---

## Waybar Indicator (Phase 1 stub)

Modules read runtime files updated by the daemon:

```json
"custom/lae-context": {
  "exec": "cat $XDG_RUNTIME_DIR/lae/context 2>/dev/null || echo 'default'",
  "interval": 1,
  "format": "{}",
  "format-default": "󰣇 default",
  "format-task": "󱓝 task:{}",
  "format-global": "󰌾 ALL DESKTOPS"
}
```

`$XDG_RUNTIME_DIR/lae/context` contains one of: `default`, `task:auth-fix`, `global`.

---

## Implementation Phases

### Phase 0: Project bootstrap (0.5 day)

- [ ] Restructure `src/lae/` package layout
- [ ] Add Typer CLI skeleton + `lae --help`
- [ ] Config defaults in `~/.config/lae/config.toml`
- [ ] XDG path helpers

### Phase 1: Task registry + Distrobox (1–2 days)

- [ ] `Task` model + SQLite persistence
- [ ] `SessionState` with `context_mode` (default on startup)
- [ ] Initialize default workspace names (ws 1–3) on daemon start
- [ ] `lae task new/list/switch/current/archive`
- [ ] `lae context default/current`
- [ ] Git clone on host
- [ ] Distrobox create per task
- [ ] `lae task terminal` (task context only; single terminal emulator, config-driven: kitty)

**Exit:** Two tasks with separate containers and repo clones; switch sets `context_mode=task`.

### Phase 2: Hyprland workspace binding + desktop isolation (2–3 days)

- [ ] Workspace slot allocator (task ranges starting at ws 10)
- [ ] Rename workspaces on task create
- [ ] `desktop_nav.py`: context-scoped go/next/prev
- [ ] `lae desktop go/next/prev` CLI + IPC
- [ ] `lae context global` / `toggle-global` / `restore` escape hatch
- [ ] `install/hypr.py`: copy `share/` → `~/.local/share/lae/`, insert source line, full-file backup + manifest
- [ ] `lae doctor` bind override checks
- [ ] `switch_task` and `context_default` dispatch Hyprland workspace
- [ ] `hyprctl` wrapper + `lae status` shows context, workspaces, windows

**Exit:** Task navigation cannot reach other tasks' desktops; escape hatch shows all; default desktops work without Distrobox; uninstall restores prior Hyprland config.

### Phase 3: Window correlation + daemon (2–3 days)

- [ ] Background daemon with UNIX socket
- [ ] Hyprland event listener
- [ ] Title prefix convention `[<task_id>]`
- [ ] Window registry reconcile loop
- [ ] CLI uses daemon when available

**Exit:** New terminals auto-register; `lae windows` is accurate.

### Phase 4: Polish + docs (1 day)

- [ ] Waybar module snippet + `lae install waybar` / `lae uninstall waybar`
- [ ] README: prerequisites, `lae install hypr`, manual rollback steps
- [ ] Manual test script / checklist (include install → use → uninstall → verify binds restored)

**Total estimate:** ~6–9 days for a working POC.

---

## Configuration Reference (defaults)

```toml
# ~/.config/lae/config.toml

[default]
workspace_range = [1, 3]         # system desktops, no distrobox
workspace_names = ["main", "comms", "aux"]

[tasks]
base_dir = "~/lae-tasks"
workspaces_per_task = 3
task_workspace_start = 10        # first slot: 10-12, second: 13-15, ...
task_workspace_stride = 3        # buffer between default (3) and tasks (10) is intentional
max_tasks = 9

[distrobox]
image = "quay.io/toolbx-images/fedora-toolbox:40"
container_prefix = "lae"

[terminal]
command = "kitty"
# task context: {command} --title "[{task_id}] terminal" -- distrobox enter ...
# default context: {command} (host shell, no wrapper)

[hyprland]
enabled = true
auto_move_tagged_windows = true
switch_task_on_window_focus = false   # if true, focusing ws 13 auto-switches to billing task
escape_hatch_keybind = "SUPER,ESCAPE"
use_global_submap = false             # Phase 2: submap for absolute jump

[daemon]
socket = "lae/daemon.sock"  # relative to XDG_RUNTIME_DIR

[install.hypr]
config_path = "~/.config/hypr/hyprland.conf"
share_dir = "~/.local/share/lae"
source_line = "~/.local/share/lae/hypr/bindings.conf"
require_sourced_last = true
allow_user_file_comments = false
```

---

## Agent Integration Hooks (future, stub now)

Reserve per task:

```
~/lae-tasks/<task_id>/.lae/
  agent-notes.md      # human + agent shared context
  agent-session.json  # future: tool state, conversation id
```

CLI stub:

```bash
lae task notes auth-fix   # opens agent-notes.md in $EDITOR
```

This aligns with the notes' goal of task-owned agent state without building an agent runtime in POC.

---

## Risks and Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Hyprland event socket disconnects | Stale window map | Reconnect loop + periodic full reconcile |
| Distrobox create is slow | Bad UX on `task new` | Show spinner; async create in daemon (Phase 3) |
| Workspace exhaustion | Can't create task | `max_tasks` config + clear error |
| Terminal doesn't support `--title` | Window correlation breaks | Configurable title flag per emulator |
| Git clone auth on host | Clone fails | Document SSH agent; support `--repo-path` for existing clone |
| Container GUI apps | Out of POC scope | Terminals only first; browser Phase 2 |
| Stock Hyprland binds bypass lae | Isolation broken | Source from ~/.local/share/lae last; doctor checks bind resolution |
| Botched install leaves bad config | User stuck | Full-file backups + restore on uninstall; no partial-line surgery |
| Omarchy multi-file hypr config | lae sourced before later overrides | Append source last in hyprland.conf; never edit Omarchy user files |
| User stuck in task context | Can't reach other desktops | Escape hatch (SUPER+ESCAPE) always available |
| Gesture/trackpad workspace swipe | May bypass lae dispatchers | Document known gap; only remap if user opts in via manifest-tracked edit |

---

## Open Questions

1. **Terminal emulator:** Standardize on kitty for POC, or detect `$TERMINAL`?
2. **Branch/worktree strategy:** Clone default branch only, or integrate `git worktree` for branch-per-task?
3. **Focus-follows-task:** Should focusing a window in global context auto-switch task context to match that workspace's task?
4. **Container lifecycle:** Stop/remove Distrobox on archive, or keep warm?
5. **Editor launch:** `cursor ~/lae-tasks/<id>/repo` on host (same files) vs inside container?
6. **Default desktop count:** Three enough, or configurable per user?
7. **Escape hatch persistence:** Should global context auto-expire after N minutes?

Recommendation for POC: host-side editor on bind-mounted path, container for shell/tests/toolchain; focus-follows-task off by default; escape hatch manual toggle only.

---

## Prerequisites (developer machine)

- Hyprland with `$HYPRLAND_INSTANCE_SIGNATURE` set
- `hyprctl` on PATH
- Distrobox + Podman (or Docker)
- Python ≥ 3.14 (per pyproject)
- Git, a terminal emulator (kitty recommended)
- Optional: Waybar for task indicator

---

## Summary

The POC proves that a thin Python control plane can sit above Hyprland and Distrobox to deliver task-centric UX:

1. **Semantic unit** = Task (not workspace, not project alone)
2. **Isolation** = Distrobox container + dedicated repo clone per task
3. **Presentation** = Hyprland workspace groups scoped by **desktop context** (default / task / global)
4. **Default desktops** = always-available host workspaces with no container
5. **Escape hatch** = temporary global context when scoped navigation is too tight
6. **Control** = `lae` CLI + daemon owning all workspace navigation via hyprctl
7. **Reversible integration** = Omarchy-style: static files in `share/` → `~/.local/share/lae/`, one `source` line in user config, full-file backups for rollback

If Phase 1–3 succeed, Phase 2+ product work (browser profiles, fuzzel switcher, agent session wiring) builds on stable task/container/workspace primitives rather than re-architecting.
