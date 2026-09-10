# Workspace switch latency and dropped keypresses

**Status (2026-09-14):** Section 1 (no pre-query) is in `/usr/bin/tsk` (rebuilt
today). Idle `tsk workspace switch` is ~12ms. The remaining ~1s stalls are
**not Hyprland/Omarchy compositor slowness** — raw `hyprctl dispatch` stays
7–15ms. They are tsk userspace: the keybind is `exec tsk` (queued on the same
hyprctl socket tsk itself floods), plus the daemon fighting with
`set_taskspace` / `ensure_workspaces`.

Measured 2026-09-14 on this machine (Hyprland 0.56.2, Omarchy quickshell bar,
`tsk 0.1.1` at `/usr/bin/tsk`).

## Comparison (same workspaces, same session)

| Command | Idle | Under tsk bar / clients load |
|---------|------|------------------------------|
| `hyprctl dispatch 'hl.dsp.focus({ workspace = "name:…" })'` | 7–15ms | 13–15ms (even 8 in a row, or during 3× `clients` / 6× `bar status`) |
| `tsk workspace dispatch N` | ~12ms | **2.2s** observed |
| `tsk workspace switch N` (SUPER+N path) | 11–18ms | **1.1s, 1.4s, 1.6s, 4.3s, 4.6s** |
| `TSK_DISABLE_HYPRLAND=1 tsk workspace switch N` | 7ms | 5–6ms (SQLite/init is not the wait) |

Stock Omarchy SUPER+N is `hl.dsp.focus({ workspace = "N" })` **inside Hyprland**
(`/usr/share/omarchy/default/hypr/bindings/tiling.lua`). No process, no
hyprctl queue. tsk replaced that with `exec tsk workspace switch N`.

Time-to-first-`hyprctl dispatch` ≈ process lifetime on slow runs: tsk is
blocked **before** the compositor switch, then one dispatch. The daemon
(`pid` of `tsk daemon run`) also issues extra dispatches:
`set_taskspace` → `ensure_workspaces` **creates a named workspace by
switching to it**, then restores. That is the “played later / never played”
path (`skip stale workspace sync` in `hyprctl.log`).

Background IPC still flooding the same socket (last 400 log lines: **217×
`clients`**): Omarchy bar `tsk bar status --json` every 2s **and on every
`workspacev2`** (6 queries each); `tsk chromium-host` ~1Hz `clients`.
`$XDG_RUNTIME_DIR/tsk/hyprctl.log` is 59MB (logging on by default). Not the
1s cause (`TSK_HYPR_LOG=0` still hit 1.1s) but it shows the flood.

## Goal

SUPER+N (and bar slot clicks) must switch in tens of milliseconds, and a burst
of switches must land on the last requested workspace — not replay stale ones
later, and not drop the last one.

## Diagnosis

Two things stack.

### 1. The keybind queries Hyprland before it dispatches

`tsk workspace switch N` reads the slot cache (good), then
`switch_workspace_for_navigation` calls `hyprctl -j monitors` to pick among
`workspace` / `focusworkspaceoncurrentmonitor`. Each SUPER+N is a **new
process**. Overlapping processes snapshot different (or the same stale) monitor
layout, then dispatch later. That is the “played later / never played”
report: an earlier process finishes its query after a later keypress and
overwrites it.

Idle cost of that extra query is ~10ms. Under load it waits behind the
pollers below and becomes hundreds of ms.

Timed here: `tsk workspace switch 2` 25ms, immediately then `switch 1` 655ms.

### 2. Hyprland IPC is busy even when you are not switching

Live `$XDG_RUNTIME_DIR/tsk/hyprctl.log` (17MB, logging **on by default**):

| Source | Pattern | IPC |
|--------|---------|-----|
| Omarchy `BarWidget.qml` | `tsk bar status --json` every 2s **and on every `workspacev2`** | 6 hyprctl calls: activeworkspace, monitors, clients, workspaces, activeworkspace, monitors |
| `tsk chromium-host` | new process ~1Hz from Chromium `sendNativeMessage` | `hyprctl -j clients` |
| Daemon `workspacev2` | listener thread, service mutex | `get_active_workspace` + possible full taskspace restore |

The bar already has live Quickshell `Hyprland.*` bindings for focus and
occupancy. Re-spawning `tsk bar status` on SUPER+N contends with the next
SUPER+N.

## Principles

- Keybind hot path: one Hyprland dispatch, no pre-query. Prefer Hyprland’s
  `workspace` dispatcher (upstream SUPER+N) over a local `list_monitors`
  workaround.
- Bar must not talk to Hyprland through `tsk` on a focus event it already
  received from socket2 / Quickshell.
- One concern per changeset.
- Fail closed: a missing slot target is an error, not a silent no-op beyond
  today’s “workspace not in cache / state” path.

## 1. Dispatch without a monitor snapshot (this change)

`switch_workspace_for_navigation` issues `workspace name:<slot>` and does not
call `list_monitors`. Delete `NavigationStrategy` / `decide_navigation_strategy`.

Hyprland already focuses the other monitor when that workspace is visible
there. We lose the tsk-only extra: “hidden workspace, last seen on monitor B,
steal it onto the focused monitor.” Restore / `set_taskspace` still use
`switch_workspace_on_monitor`.

## 2. Bar: do not spawn `tsk bar status` on workspace focus

`BarWidget.qml` `onRawEvent` for `workspacev2` / `focusedmon*` should bump
`hyprRev` only (live bindings already repaint). Keep `tsk bar status` for
`state.rev` (taskspace / slot map) and a slow fallback timer.

## 3. Later, separately

- Chromium native host: stop a 1Hz `get_clients` spawn (use daemon/socket2 or
  debounce inside one long-lived host).
- Default-off or rotate `hyprctl.log` (17MB append on every IPC).
- Daemon: do not `get_active_workspace` on same-taskspace `workspacev2`;
  debounce `sync_window_registry`.

## Out of scope

- Rewriting SUPER+N to native Hyprland binds generated on taskspace change
- Daemon actor / dropping the service mutex
- Animation settings
- Taskspace switch (`set_taskspace`) latency

## Success criteria

1. `switch_workspace_for_navigation` does not call `list_monitors` (section 1).
2. A rapid SUPER+2 then SUPER+1 ends on slot 1; slot 2 does not apply after.
3. After section 2 is installed: `workspacev2` does not spawn `tsk bar status`.
4. Idle `tsk workspace switch N` stays on the order of one `hyprctl dispatch`
   plus process spawn (tens of ms), not a second, when not colliding with
   Chromium-host / leftover bar polls.

## Reuse survey

Extending `switch_workspace_for_navigation` in `crates/tsk-core/src/hyprland.rs`
(the function every keybind already uses via `workspace_dispatch` / slot
cache). Not adding a second navigation path. Bar/host/daemon work is later
sections, not this changeset.
