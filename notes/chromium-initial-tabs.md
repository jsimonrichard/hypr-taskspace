# Chromium initial tabs (new taskspaces)

> **Implementation status (2026-09-07):** Landed in this changeset. New taskspaces no
> longer inherit another task's snapshot. First Chromium launch with no saved
> session opens `.tsk/repo.toml` `[browser].default_tabs`, else the repo browse
> URL, else `chrome://newtab`.

## Goal

A new taskspace's first Chromium window is this task's tabs only: configured
defaults, else the current repo's browse page, else one new tab. Never another
task's snapshot.

## Principles

1. No silent fallback. No Hypr match → do not write a session.
2. Hyprland is the attribution source; the extension only supplies tab lists
   for windows already on that task.
3. One initial-URL resolver for every first launch. Do not special-case GitHub
   in the launcher — treat it as one derived browse URL.
4. Two concerns, one user-visible fix: stop the leak and always pass explicit
   URLs so a shared Chromium profile cannot restore someone else's session.

## Work

1. Delete `fallback_current_task_windows` and the unmatched one-window guess in
   `assign_session_windows`. Flip tests that encoded those fallbacks.
2. Add `[browser].default_tabs` on `RepoConfig`, convert clone remotes to
   browse URLs in `vcs`, and pass those URLs (or `chrome://newtab`) when
   `restore_pending` is a no-op.

## Out of scope

Isolated-profile behavior, Firefox, changing archive/restore of a correctly
attributed snapshot, extension rewrite.

## Success criteria

1. New task + first Chromium launch → configured / derived / NTP tabs, never
   another task's URLs.
2. A task that actually had Chromium on its workspaces still restores those
   tabs after close/relaunch and after archive/restore.
3. Empty `default_tabs` / unknown remote → `chrome://newtab`.
4. `tsk chromium status` on a fresh task shows no stolen Saved session.
