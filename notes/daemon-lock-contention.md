# Daemon lock contention and concurrency options

> Engineering notes from a 2026-07 discussion. Not scheduled for immediate work — captures the problem, what was already fixed, remaining offenders, and options if we revisit concurrency later.

## Problem observed

After archiving a git-backed task, the TUI would often (not always) briefly show that the daemon was unreachable. Initial suspicion was a daemon crash/restart; investigation showed the daemon was still running.

## Root cause

The daemon wraps all session state in a single `Arc<Mutex<TaskService>>`. Every RPC handler acquires that lock for the full duration of the request. Archiving (and similar operations) can take hundreds of milliseconds to several seconds because they call:

- Hyprland IPC (`set_taskspace`, `close_task_windows`, workspace setup)
- distrobox subprocesses (`stop_container`, etc.)
- git/jj subprocesses (worktree detach/reattach)
- filesystem work
- optional hook scripts (`.tsk/on-start.sh`)

Meanwhile the TUI runs a background health check (`AsyncDaemonChecker`) that pings the daemon every ~5 seconds with a **300ms timeout**. If archive (or anything else) held the lock longer than that, the ping queued behind the slow work, timed out, and the TUI flipped to “daemon not running” — a false negative.

Relevant code:

- `crates/tsk-core/src/daemon/server.rs` — `dispatch()` locks `TaskService` for every RPC; per-connection handler threads all contend on the same mutex
- `crates/tsk-core/src/daemon/client.rs` — `is_daemon_running()` / `PING_TIMEOUT`
- `crates/tsk-tui/src/daemon_check.rs` — background ping loop

The Hyprland window event listener also grabs the same lock on every `openwindow` / `closewindow` / `movewindow` event to run `sync_window_registry()`.

## Architectural constraint

`TaskService` uses a **load entire state → mutate → save entire state** pattern. `Registry::save_state` rewrites the full `tasks` and `windows` tables (delete-all + reinsert), not row-level updates. That means:

- A global `RwLock` with concurrent readers is **unsafe** unless writes are wrapped in a SQLite transaction (or the save path is redesigned).
- Per-task locks are awkward because `current_task_id`, taskspace context, window registry, and navigation memory are global cross-cutting state.

See `crates/tsk-core/src/registry.rs` (`save_state`).

## Fixes already applied (2026-07)

| Change | Location | Effect |
|--------|----------|--------|
| `ping` bypasses the service lock | `daemon/server.rs` `dispatch()` | Health checks respond even during long operations |
| Archive teardown outside lock | `prepare_archive` / `run_archive_teardown` / `complete_archive` | Window close, distrobox stop, git detach no longer block other RPCs |
| Ping retries (3×, 50ms gap) | `daemon/client.rs` `is_daemon_running()` | Single slow response doesn’t flip daemon status |
| TUI marks daemon running after successful archive | `tsk-tui/src/app.rs` | Avoids stale “stopped” after a successful RPC |

**Still locked during archive of the active task:** `prepare_archive` calls `set_taskspace` before teardown is released (state mutation + heavy Hyprland work must stay paired today).

Restart the daemon after pulling these changes.

## Remaining code paths that hold the lock across slow I/O

### High impact (same class as the original archive bug)

| Path | Slow work while locked |
|------|------------------------|
| `delete_task` | `set_taskspace` (if active task), `close_task_windows`, distrobox stop/remove, git/jj checkout removal, data dir deletion |
| `restore_task` | `reattach_task_checkout` (git/jj), `start_task_container`, `run_on_restore_after_restore` (hook script + Hyprland prep) |
| `create_task` | `provision_task_checkout` (git worktree / jj workspace), optional `switch_task` → `set_taskspace`, `setup_task_workspaces`, `run_on_create_after_create` (hook) |
| `prepare_archive` (active task only) | `set_taskspace` before teardown |

### Medium impact

| Path | Slow work while locked |
|------|------------------------|
| `switch_task`, `context_default`, `set_context` (task) | `set_taskspace` — multi-monitor Hyprland setup; intentional for consistency but blocks other RPCs |
| `open_terminal` | `ensure_task_checkout_ready` (can run full git/jj relink), terminal spawn |
| `preview_task_teardown` | `hyprland::get_clients()`, `container_exists` (distrobox) |

### Background (not user-initiated RPCs, same mutex)

| Path | Slow work while locked |
|------|------------------------|
| Hyprland window listener | `sync_window_registry` on every open/close/move — `get_clients()` + full DB snapshot save |
| Daemon startup (once) | `provision_default_workspaces`, initial `sync_window_registry` |

### Fast / low concern

These hold the lock but are mostly SQLite reads or in-memory state updates:

- `get_state`, `tasks_for_menu`, `resolve_task`, `taskspace_label`
- `remember_workspace_go` / `remember_workspace_goto` (state sync only; Hyprland workspace *switch* is done separately by CLI `workspace_dispatch`, which bypasses the daemon lock)
- `reset_navigation_layout`

## Options for finer-grained concurrency (by effort)

### Tier 1 — I/O outside the lock (incremental)

**Effort: ~1–2 days. Risk: low.**

Extend the archive pattern: short locked prepare/finalize phases, slow work outside the lock.

Candidates:

1. `delete_task` (biggest remaining user-visible blocker)
2. `restore_task`
3. `create_task` (slowest RPC — checkout provisioning + hooks)
4. Read-only RPCs that only need a snapshot (`tasks_for_menu`, `get_state`, etc.)

Does **not** require architectural change. Solves most false “daemon down” cases and TUI blocking.

### Tier 2 — Debounce window registry sync

**Effort: ~0.5–1 day. Risk: low.**

Coalesce Hyprland window events (e.g. 50–100ms) so `sync_window_registry` runs once per burst instead of per event. Reduces background lock churn during normal desktop use. Independent of Tier 1.

### Tier 3 — `RwLock` for read-heavy paths

**Effort: ~3–5 days. Risk: medium.**

Allow concurrent reads for TUI polling while writes serialize.

**Prerequisite:** wrap `save_state` in a SQLite transaction (or move to incremental SQL updates). Without that, concurrent reads can observe a half-written snapshot (delete-all + reinsert).

Benefit is modest for a single-user daemon: most hot-path RPCs (`switch_task`, `workspace_*` via daemon, `set_context`) are writes anyway.

### Tier 4 — Actor / single-writer channel

**Effort: ~1–2 weeks. Risk: medium (large refactor, clean model).**

Replace the mutex with a state-owning thread processing a command queue. RPC threads send `Command` and await `Reply`. Long I/O becomes `Prepare → spawn blocking work → Complete` messages.

Touches `daemon/server.rs`, most of `service.rs`, and tests. Good sweet spot if we add more background work or multiple concurrent clients.

### Tier 5 — Row-level DB + WAL + transactions

**Effort: ~2–4 weeks. Risk: high.**

Stop wholesale `SessionState` load/save. Per-operation SQL, SQLite WAL, optional version column for optimistic concurrency. Foundation for safe concurrent reads and narrower write scopes. Largest architectural change.

### Tier 6 — Per-task locks

**Effort: ~1–2 weeks. Risk: high, benefit questionable.**

Archiving task A shouldn’t need to block switching to task B — in theory. In practice taskspace context, window registry, and session keys are global. Multiple locks + ordering rules → deadlock risk. Skip unless there is a concrete multi-task concurrency requirement.

## Recommended priority (when we pick this up)

1. **Tier 1:** `delete_task` split (mirror archive), then `restore_task`, then `create_task`
2. **Tier 2:** debounced window registry sync
3. Revisit **Tier 4** only if we need a principled concurrency story beyond “don’t hold the lock during subprocess/Hyprland work”

Tier 3 and Tier 5 are only worth it if we expect heavy concurrent read load or multi-process access to `state.db`.

## Related work in the same conversation arc

Other features landed around the same time (for context, not lock-specific):

- Archive / restore tasks (core, daemon, CLI, TUI)
- TUI keybindings: `r` restore (Archived panel), `R` refresh
- jj workspace restore after `forget` (`relink_forgotten_jj_workspace`)
- git worktree detach on archive / relink on restore
- Task hooks: shared `on_start` (default `.tsk/on-start.sh`), optional `on_create` / `on_restore` overrides
- Repo unregister in TUI no longer deletes `.tsk/repo.toml`

## Key takeaway

For a single-user desktop daemon, the global mutex is less “wrong” than **holding it across subprocess, Hyprland, and git work**. The archive fix addressed one instance of that pattern; `delete_task`, `restore_task`, and `create_task` are the main remaining offenders, plus the window event listener as ongoing background contention.
