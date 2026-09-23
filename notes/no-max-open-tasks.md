# No limit on open (active) tasks

**Status (2026-09-25):** Deferred — noted from conversation; not implemented.

## Goal

Do not cap how many non-archived tasks a user may have open. Creating a task
should not fail because of an artificial `max_tasks` count.

## Why

`max_tasks` (default **9**) is enforced in `TskService` create path and rejects
with `"Maximum task limit (N) reached"`. That was a PoC guard for workspace
exhaustion; in practice users want unbounded open tasks and will archive when
they choose.

## Principles

- **No silent fallback.** Removing the cap means deleting the gate, not setting
  a huge default that still rejects.
- Prefer deleting `max_tasks` (config field, defaults, docs) over keeping a
  knob nobody should need. If a safety valve is still wanted later, make it an
  explicit opt-in — not a default limit.
- One path: create still fails for real reasons (VCS, paths, id collision),
  never for “too many active tasks.”
- Hyprland workspace slot count (`default_workspace_count` / SUPER+1..0) is a
  separate concern from how many *tasks* exist; do not conflate them when
  removing this gate.

## Reuse survey (when implementing)

Remove / stop using:

- Create-time check in `crates/tsk-core/src/service.rs` (`active_count >=
  self.config.max_tasks`)
- `TskConfig::max_tasks` and `[tasks] max_tasks` parsing in
  `crates/tsk-core/src/config.rs` (defaults and example config snippets)
- Mentions in `notes/poc-plan.md` / user docs that advertise the limit

UI (Omarchy overlay, TUI, CLI) only needs to stop receiving that error; no
separate cap in frontends.

## Out of scope

- Changing archive-on-boot or other lifecycle policies.
- Raising Hyprland workspace counts or keybind slots.
- Soft warnings (“you have N open tasks”) unless asked for later.

## Success criteria

1. Creating the 10th (and Nth) active task succeeds when other invariants hold.
2. Config no longer documents or requires `max_tasks` (or treats a present
   value as unused and removable).
3. No frontend shows a “maximum task limit” message from tsk.
