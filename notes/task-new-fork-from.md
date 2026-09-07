# `tsk task new` fork-from options

## Goal

Let `tsk task new` create a linked git worktree / jj workspace from an explicit
revision, from a named jj workspace (or tsk task checkout), or from the checkout
the user is standing in — instead of always using `trunk()`/`main` (jj) or the
source HEAD (git).

## Principles

- Logic lives in `tsk-core`; `tsk-cli` only exposes flags. The daemon carries
  the fork spec on `create_task`.
- **Default is unchanged:** jj still uses `trunk()`/`main`; git still uses the
  source checkout `HEAD`.
- **Fail closed.** Unknown rev, missing current checkout, scratch/`--no-worktree`
  plus a fork flag, or a workspace that is not in the source repo → error. No
  silent fall-back to `main`.
- One resolution path for “current”: find a checkout, then ask that checkout for
  its live `@` / `HEAD`. Taskspace vs registered repo vs raw clone differ only
  in how the checkout path is found.
- `--from` is a VCS revset / commit-ish. Task or jj workspace identity is
  `--from-workspace` / `--from-current` (revsets like `t231590d8@` still work
  via `--from`).
- `jj workspace add -r` means **parents of the new working-copy commit**.
  `--from` / `--from-current` / `--from-workspace` all resolve to that parent.

## Reuse survey

Extending, not replacing:

- `create_jj_workspace(..., revision)` and `resolve_jj_default_base` in
  `crates/tsk-core/src/vcs.rs`
- `create_git_worktree` (add an optional start-point)
- `TaskRepoOptions` / `provision_task_checkout` / `create_task` daemon params
- `detect_vcs_root`, `jj_template`, `jj_list_workspaces`, `lookup_task`

Not extending the TUI new-task form (stays on the default fork). A second
change can add that UI.

`--from-current` uses any detected git/jj root (same as `TaskRepoSource::Auto`).
`TSK_TASK_ID` is a fallback only when cwd has no VCS root.

## Numbered work sections

1. **Core fork spec** — `ForkFrom` on `TaskRepoOptions`; resolve to a revision;
   pass it into `create_linked_checkout`.
2. **CLI + daemon** — `--from`, `--from-current`, `--from-workspace`; wire
   through `create_task` JSON.
3. **Tests + README** — jj/git resolution, fail-closed cases, user-facing
   examples.

## Out of scope

- TUI new-task form fields
- Changing the default away from `trunk()`/`main`
- Remote clone / `--branch` on `task new`
- Sharing a working-copy change (`jj edit`) across taskspaces

## Success criteria

1. `tsk task new x --from <rev>` creates the linked checkout with that rev as
   the git start-point / jj `-r` parent.
2. `tsk task new x --from-current` inside a task checkout (or a detected
   git/jj repo) forks from that checkout’s live `@` / `HEAD`, not `trunk()`.
3. `tsk task new x --from-workspace <name>` forks from `name@` (jj) or from
   that tsk task’s checkout when it belongs to the same repo.
4. Scratch, `--no-worktree`, unknown rev, and “not in a repo” with
   `--from-current` all error with a message that names the path / rev / id.
5. Existing `tsk task new` with no fork flags keeps today’s base.
