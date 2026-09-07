# Overlay command errors

## Goal

Every user-initiated binary the Omarchy `tsk.taskspace` overlay starts (`tsk` or
`omarchy-file-select`) must be able to show its failure through the existing
error dialog, including `tsk task restore` failing on a stale jj working copy.

## Principles

- One error path: `showCommandError` / `queuePendingError` / `errorDialog`.
- No fire-and-forget for commands whose failure the user can hit.
- A queued error survives `open()`; a second summon must not clear it.
- Capture command output as the process writes it, not only from a collector
  that may still be empty in `onExited`.
- Background refreshes (`task list`, `repo list`) stay inline; they must not
  pop the dialog on every `state.rev` bump.

## 1. Hold and present command errors

- Stop `applyPendingError` from calling `clearCommandError` when nothing is
  queued (that wipes a dialog shown by a prior `open()`).
- Hold a shown error across `open()` until a new action or a successful
  command clears it.

## 2. Wait on every user `tsk` command

- Route switch / default through `actionProc` (same dismiss → OSD → reopen on
  failure path as restore).
- Delete `Util.execDetached` from the overlay.
- Generalize restore’s failure title/reopen onto `actionKind`.

## 3. Read stderr on the other binaries

- Accumulate `actionProc` stdout/stderr with `SplitParser`.
- `repo list`: capture stderr; show it on the Repos empty state.
- `omarchy-file-select`: capture stderr; queue the dialog when it failed with
  output (cancel with no output stays silent).
- Container create: open the error dialog on failure (log is already captured).

## Out of scope

- Bar widget `bar.run` (`tui-launch`, `workspace switch`) — no error dialog on
  the bar.
- Changing jj stale-working-copy recovery in `tsk-core`.
- Removing the restore/switch OSD.

## Success criteria

1. Restore failure reopens the overlay with the dialog body containing the
   `tsk`/`jj` message (not only a generic title).
2. Switch and default failures do the same.
3. Create / rename / archive / delete / repo add/remove already do, or now do.
4. Folder-picker cancel does not look like an error; a picker crash does.
5. Repo list failure is visible on the Repos tab.
6. `Taskspace.qml` contains no `Util.execDetached`.
