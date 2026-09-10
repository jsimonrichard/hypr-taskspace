# Sibling checkouts (`tsk checkout add`)

**Status (2026-09-10):** CLI create landed. Archive/restore now discovers
detached git siblings by the leftover `tsk-<id>-<suffix>` branch (a live
`.git` is gone after detach). TUI is out of scope.

---

## Goal

Add a concise CLI that creates a second git worktree / jj workspace **inside
the current task home**, named as an extension of the task id, without creating
a new tsk taskspace.

Example, standing in `t74c8e14d@` at
`~/tsk-tasks/t74c8e14d/workspace/hypr-taskspace`:

```bash
tsk checkout add review
# jj name:  t74c8e14d-review   (jj shows t74c8e14d-review@)
# git branch: tsk-t74c8e14d-review
# path:     ~/tsk-tasks/t74c8e14d/workspace/hypr-taskspace-review
```

`cd "$(tsk checkout add review)"` must work: stdout is the dest path only.

## Principles

- **Not a new task.** `task.repo_path` stays the primary checkout.
  `tsk task editor` / terminal / archive identity do not switch.
- **Not Hyprland.** `tsk workspace` stays compositor slots. This command is
  `tsk checkout …`.
- **Logic in `tsk-core`.** CLI only parses flags and prints the path.
- **One create path.** Reuse `create_linked_checkout` + `ForkFrom`. Do not
  add a second jj/git add implementation.
- **Suffix is the argument.** `review` → `{task_id}-review` and
  `{repo_label}-review`. Do not accept a full id (that would double-prefix).
- **Default fork is `--from-current`** (the checkout you are in: `@` /
  `HEAD`). `--from REV` overrides. `task new` keeps its trunk/HEAD default.
- **Fail closed.** No current task, scratch task, missing VCS, invalid
  suffix, dest exists but is not a workspace → error that names the path /
  id / suffix. No fallback to `task new` or to the source repo checkout.
- **Task-owned by location.** Extra checkouts live under
  `<task-home>/workspace/` and are discovered by scanning that directory.
  No new `Task` field and no daemon RPC for create.
- **Archive/delete/restore must see them.** Creating siblings without
  tearing them down leaves orphaned jj workspaces / git worktrees. Cleanup
  generalizes to every VCS root under `workspace/`, not only
  `task.repo_path`.

## Reuse survey

Extending, not replacing:

- `create_linked_checkout` / `create_jj_workspace` / `create_git_worktree`
  in `crates/tsk-core/src/vcs.rs` — dest + workspace name are already
  arguments; git already prefixes `tsk-` onto the name.
- `ForkFrom::Revision` / `ForkFrom::Current` and
  `ForkFrom::resolve_revision` in `task_repo.rs`.
- `linked_checkout_path` / `task_workspace_dir` / `repo_label` /
  `is_managed_task_checkout` in `task_paths.rs`. Sibling dest is the same
  workspace dir with `-{suffix}` on the folder name.
- `task_source_repo_path` — always the source for `jj workspace add` /
  `git worktree add`, even if cwd is already a sibling.
- `task_home_for_checkout` / `jj_restore_checkout_key` — already key
  restore metadata by checkout folder name, so siblings get their own
  sidecar without a schema change.
- `detach_task_checkout` / `reattach_task_checkout` / `remove_task_checkout`
  in `task_cleanup.rs` — today they only touch `task.repo_path`. Generalize
  to all VCS roots under the task workspace dir.
- `resolve_current_or_named_task` in `tsk-cli` — last-resort task id when
  cwd is not under a task home.

Not extending:

- `tsk workspace` (Hyprland).
- `tsk task new` (new taskspace + primary checkout).
- TUI new-task form.
- Task registry / `source_repo_path` schema.

Why not only `task new --from-current`: that creates Hyprland slots, a
task id, on-start, browser session, etc. The request is a second WC in
the **same** task home.

## Numbered work sections

1. **Path and name helpers** — `sibling_checkout_path`,
   `sibling_workspace_name`, suffix validation in `task_paths.rs`.
2. **Create in core** — `add_sibling_checkout` (task + suffix + optional
   rev + cwd) resolves source, dest, name, revision, then calls
   `create_linked_checkout`. Task resolution: cwd under
   `~/tsk-tasks/<id>/workspace/` first, else `TSK_TASK_ID`, else daemon
   current task.
3. **CLI** — `tsk checkout add <suffix>` and optional `--from REV`.
   Stdout is the dest path. Human detail (jj name / git branch) on stderr
   if we need it; keep the happy path one line.
4. **Lifecycle** — archive / restore / delete iterate every git/jj root
   under `<task-home>/workspace/` (including the primary). Scratch
   (`workspace/` itself) stays a single checkout.
5. **Tests + README** — jj and git create, dest/name, fail-closed cases,
   cleanup sees the sibling. One README example next to `task new`.

## Out of scope

- TUI / overlay control for adding a sibling.
- `tsk checkout list` / `remove` (use `jj workspace list` /
  `git worktree list` for now).
- Opening editor/terminal in the sibling (`tsk task editor` stays on
  `task.repo_path`).
- Recording extra checkouts on the `Task` row.
- Changing `task new` defaults.
- Distrobox / container bind-mounts for the sibling.

## Success criteria

1. `tsk checkout add review` inside this task creates
   `…/t74c8e14d/workspace/<repo>-review` and a jj workspace named
   `t74c8e14d-review` (or git branch `tsk-t74c8e14d-review`).
2. Command stdout is exactly that dest path (scriptable `cd`).
3. `--from <rev>` sets the git start-point / jj `-r` parent; with no
   `--from`, the parent is the cwd checkout’s live `@` / `HEAD`.
4. Scratch, no current task, invalid suffix, and “dest exists but is not
   a workspace” error with the path / id / suffix in the message.
5. Re-running the same add on an already-valid sibling is idempotent
   (prints the existing path).
6. Archive detaches the sibling jj/git registration; restore reattaches
   it; delete removes it. Primary checkout behavior is unchanged.

## Decision

Default fork is **`--from-current`** (2026-09-10). `--from REV` is the
only override; there is no `--from-current` flag.
