---
name: read-handoff
description: >
  Read and follow workspace/HANDOFF.md for the current tsk task. Use when
  TSK_TASK_ID or TSK_HANDOFF is set, when the user mentions a task handoff /
  HANDOFF.md / tsk instruct, or when starting work inside a tsk task checkout.
  Soft-gate: if the handoff file is missing, do not invent one — ask or use
  tsk task instruct / Split. Works with or without an orch-managed lane.
---

# Read HANDOFF first (tsk task)

tsk’s structured task contract is **`HANDOFF.md`**, not freeform notes and not
orch-only `AGENT_BRIEF.md`.

| Signal | Meaning |
|--------|---------|
| `$TSK_HANDOFF` | Absolute path (set on task spawns; file may be missing) |
| `$TSK_TASK_ID` | Opaque task id under `~/tsk-tasks/<id>/` |
| `tsk task handoff` | Prints path; appends `(missing)` if absent |
| `tsk task handoff --validate` | Fail closed if missing/invalid |

Canonical path: `~/tsk-tasks/<id>/workspace/HANDOFF.md` (sibling to a linked
repo folder; inside `workspace/` for scratch).

`.tsk/agent-notes.md` is a freeform scratchpad — **not** the contract.

## Resolve and read

```bash
# Prefer the env stamped at spawn
test -n "$TSK_HANDOFF" && test -f "$TSK_HANDOFF" && cat "$TSK_HANDOFF"

# Or ask tsk
tsk task handoff              # path (+ missing)
tsk task handoff --validate   # non-zero if invalid
```

If `TSK_TASK_ID` is set but the file is missing: do **not** invent a handoff.
Report the gap; the user (or orch / overlay Split) can run:

```bash
tsk task instruct --from ./brief.md
# or at create time:
tsk task new <name> --from-current --handoff ./brief.md
```

## After a validating HANDOFF exists

1. Obey **Scope** and **Out of scope**.
2. Meet **Success criteria** observably. Report gaps (house-rule 6).
3. Treat **Principles** / **Handoff notes** as constraints when present
   (optional sections in tsk).
4. Run the repo gate (`.claude/gate.sh`) before push when configured.

## Required headings (tsk validate)

Must exist; Goal and Success criteria must be non-empty:

- Goal
- Scope
- Out of scope
- Success criteria
- Constraints

Optional: Principles, Handoff notes.

## Orch lanes

If `~/tsk-tasks/$TSK_TASK_ID/.tsk/orch.json` exists, this task was also
provisioned by orch — still use **this** HANDOFF path (`TSK_HANDOFF`), not
legacy `.tsk/AGENT_BRIEF.md`, once orch has migrated. Until then, orch may
still write AGENT_BRIEF; prefer HANDOFF when both exist after cutover.
