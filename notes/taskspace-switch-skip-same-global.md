# Taskspace switch: skip same global last-workspace

**Status (2026-09-23):** Deferred — noted from conversation; not implemented.

## Goal

When switching taskspaces while already on a global workspace, if the target
taskspace’s remembered last workspace is that **same** global workspace, land on
the target’s last **non-global** workspace instead of staying on the shared
global.

## Why

Global slots (e.g. `"1"`) are shared Hyprland workspaces across every taskspace.
Leaving task A on `"1"` and entering task B whose `last_workspace` is also `"1"`
looks like a no-op: you stay on the same screen and never enter task B’s own
slots. Prefer the last place that was actually task-local in B.

## Principles

- One focus-resolution path for intentional taskspace switches
  (`set_taskspace` / `sync_monitors_to_taskspace` / `remembered_focus_slot`).
  Do not special-case default vs task identity beyond “global vs not.”
- Still treat globals as ordinary last-active locations for **within**-taskspace
  navigation and for switches that do **not** start on a global (current
  `remembered_focus_slot` behavior and tests stay the default).
- Fail closed only if there is no non-global to restore: then keeping the shared
  global (or the existing primary-slot default) is correct, not a silent lie.
- Derive “same global” from the authoritative slot / workspace name helpers in
  `workspaces.rs` (`is_global_workspace_slot` / `is_global_workspace_name`), not
  a second list of magic names.

## Reuse survey (when implementing)

Extend, do not fork:

- `remembered_focus_slot` and `focus_last_workspace` in
  `crates/tsk-core/src/workspace_nav.rs` — today’s restore target.
- `sync_monitors_to_taskspace_inner` — multi-monitor path that also reads
  `last_workspace` for the focused monitor fallback.
- `state.last_workspace` (and likely a sibling map or richer memory) in session
  state — today one slot per taskspace key; this feature needs the last
  **non-global** slot as well when the remembered slot is global.
- `is_global_workspace_slot` / `primary_task_workspace_slot` in
  `crates/tsk-core/src/workspaces.rs`.

## Proposed behavior

1. Source focus is a global workspace **and**
2. Destination `last_workspace` resolves to that same global workspace →
3. Focus destination’s last remembered **non-global** slot (else existing
   fallback: primary non-global / slot 1).

Otherwise keep current restore behavior.

## Out of scope

- Changing within-taskspace SUPER+N / next-prev when already in a taskspace.
- External Hyprland focus that was already aimed at a specific workspace name
  (`sync_taskspace_from_external` lands where the user clicked).
- Latency work in `notes/workspace-switch-latency.md`.

## Success criteria

1. On global `"1"` in task A, with task B’s last memory also `"1"` and a prior
   non-global (e.g. slot 2) → switch to B lands on B’s non-global, not `"1"`.
2. On global `"1"` in A, with B’s last memory a non-global → still that
   non-global (unchanged).
3. On a non-global in A → switch still uses B’s remembered last (including
   global) unchanged.
4. No non-global history for B → stay on / restore the shared global (or
   primary-slot default), with no invented workspace.
5. Existing `remembered_focus_slot_*` unit tests still describe the non-switch
   cases; add a switch-from-same-global case.
