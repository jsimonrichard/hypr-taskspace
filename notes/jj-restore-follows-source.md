# Restore linked checkouts from the source VCS

## Goal

Archive restore of a jj-backed task must re-register a jj workspace. It must
never call `git worktree add` just because the checkout looks like a git
worktree (colocated leftover `.git` file, or no `.jj` after forget).

## Principles

- The registered source repo is authoritative for VCS kind (`vcs_kind_at`).
  Checkout markers are a fallback only when there is no source.
- A missing managed checkout is not success. Recover leftover
  `.{id}-relink-tmp` / `.{id}-git-relink-tmp`, then recreate the link.
- A jj workspace name that is registered but whose root is missing or does not
  match the checkout is a ghost: forget it, then relink. Do not treat a failed
  `jj workspace list` as forgotten (existing rule).
- Fail closed: git worktree add of `tsk-<id>` on a jj repo is an invariant
  break, not a fallback.

## Reuse survey

Extending `reattach_linked_checkout` in `crates/tsk-core/src/vcs.rs`. Already
used by `reattach_task_checkout` / `ensure_task_checkout_ready`. Reuses
`recover_relink_backups`, `relink_forgotten_jj_workspace`,
`forget_jj_workspace`, `reconnect_jj_workspace`, `vcs_kind_at`. Not extending
fork-from or the workspace-list `root` template (separate jj 0.44 concern).

## 1. Source kind wins

One match on source kind (else checkout kind). Jj source always takes the jj
reattach path, including when the checkout has only a `.git` file or is
missing.

## 2. Live vs ghost jj workspace

Live: listed root matches the checkout and `.jj` is present, **or** the name is
registered, root is unrecorded, and `.jj` is present. Otherwise forget the name
if registered, then `relink_forgotten_jj_workspace`.

## Out of scope

- Rebuilding / installing the Sep 4 prod daemon (user must pick up the binary).
- Changing `jj workspace list -T root` parsing for fork-from.
- praxis-books or other archived tasks unless they share this path.

## Success criteria

1. Colocated jj source + checkout that has a `.git` file and no `.jj` restores
   as a jj workspace (`git worktree add` is not invoked).
2. Missing checkout with leftover `.{id}-relink-tmp` is recovered and relinked.
3. Existing git detach/reattach and jj detach/reattach tests still pass.
4. Named ghost workspace (registered, dest deleted) can be restored.
