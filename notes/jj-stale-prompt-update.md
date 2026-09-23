# Prompt to run `jj workspace update-stale` on stale errors

**Status (2026-09-24):** Deferred — noted from conversation; not implemented.

Related: `notes/overlay-command-errors.md` (surfaces stale jj messages in the
Omarchy overlay; explicitly left recovery out of scope).

## Goal

When a user-facing action fails because a jj workspace is stale (primarily in
the Omarchy `tsk.taskspace` shell UI), prompt whether the tsk runtime should
dispatch `jj workspace update-stale` on that checkout so the original action
can be tried again.

## Why

Stale working-copy errors are recoverable but opaque: the overlay already shows
the jj message, yet the user still has to leave the UI, run
`jj workspace update-stale` in the right checkout, and re-invoke the action.
`reconnect_jj_workspace` in `tsk-core` already wraps that command for
reactivation paths; the missing piece is an explicit, consentful recovery from
the error dialog.

## Principles

- **Consent first.** Never auto-run `update-stale` from an error path; offer a
  prompt (reuse `confirmDialog` or extend `errorDialog` with an action).
- Logic for detecting “this is a stale-workspace failure” and for invoking
  update-stale lives in **`tsk-core`** (and a thin CLI/daemon entry if needed).
  The Omarchy overlay only presents the prompt and re-runs the failed action —
  no second copy of jj invocation in QML.
- One recovery helper: extend or call `reconnect_jj_workspace` /
  `ensure_task_checkout_ready` rather than a parallel `jj` spawn.
- Fail closed: if update-stale fails, show that error; do not pretend the
  original action succeeded. If the failure text is not recognizably stale,
  keep the plain error dialog (no bogus prompt).
- After a successful update-stale, **retry the same user action** once (restore,
  switch, create, …), not only dismiss the dialog.

## Reuse survey (when implementing)

Extend:

- `reconnect_jj_workspace` / `ensure_checkout_ready` /
  `ensure_task_checkout_ready` in `crates/tsk-core/src/vcs.rs`
- Omarchy overlay error path in `share/omarchy-plugin/Taskspace.qml`
  (`showCommandError` / `queuePendingError` / `errorDialog` / `confirmDialog`)
  and whatever `Model.commandFailureTitle` / stderr capture already feeds it
- Any `TskError` variants that already carry checkout path — prefer a typed
  stale signal over string-matching jj stderr in QML; if classification stays
  in core, return a structured “recoverable: update-stale” alongside the
  message

## Proposed behavior

1. User action fails; stderr / error is classified as jj workspace stale.
2. UI prompts: offer to run update-stale (and cancel).
3. On confirm: tsk runs update-stale for the implicated checkout.
4. On success: retry the original action once; on failure: show the new error.

## Out of scope

- Auto-healing every jj call inside the daemon without UI consent.
- Changing when archive/reattach already call `reconnect_jj_workspace`.
- Bar widget paths that have no error dialog (`notes/overlay-command-errors.md`).
- Non-jj VCS.

## Success criteria

1. Omarchy overlay: a stale failure on restore (or similar) offers update-stale;
   confirm → update-stale → original action retried.
2. Decline / cancel leaves the error as today (no silent mutation).
3. Non-stale failures never show the update-stale prompt.
4. CLI/TUI may share the same core helper later; Omarchy UI is the first
   surface.
