# Cursor integration

How tsk opens Cursor and how that will evolve when a task has multiple repos.

## Current behavior

When a task is **created** or **restored** from archive, tsk runs `.tsk/on-start.sh` by default (no `repo.toml` entry required). Use `TSK_TASK_HOOK` (`create` or `restore`) inside the script if you need different behavior per event. Scratch tasks skip on-start.

Optional settings in `.tsk/repo.toml`:

```toml
# on_start_monitor = "eDP-1"
# on_create = ".tsk/on-create.sh"   # override for create only
# on_restore = ".tsk/on-restore.sh" # override for restore only
```

Default script behavior (see the repo’s `.tsk/on-start.sh`):

```bash
# Prefer the tsk launcher — respects Distrobox isolation when the task was created with --container
tsk task editor "$TSK_TASK_ID"
```

Legacy / host-only:

```bash
cursor "$TSK_TASK_REPO"
```

When `TSK_CONTAINER_ISOLATION=1`, on-start scripts should launch via Distrobox (`distrobox enter --name "$TSK_CONTAINER_NAME" -- cursor "$TSK_TASK_REPO"`) or `tsk task editor`. Distrobox integrates GUI apps with the host Wayland session.

`TSK_TASK_REPO` is the task-specific path under `~/tsk-tasks/<id>/workspace/<repo-folder-name>` (or `~/tsk-tasks/<id>/workspace` for scratch tasks). Always use this path — not the canonical repo location elsewhere on disk — so Cursor scopes agent conversations to the task checkout.

Task-owned agent metadata (notes, future session index) lives at the task home:

```
~/tsk-tasks/<id>/.tsk/
  agent-notes.md
  agent-session.json   # future
```

Cursor conversation content itself stays in Cursor's user data (`~/.config/Cursor/User/`, `~/.cursor/projects/`). Project `.cursor/` under a checkout is for shareable config (rules, hooks, MCP), not conversation storage.

## Multiple repos per task (future)

When a task has more than one registered checkout under `workspace/`, opening a single repo folder would split agent conversations across separate Cursor workspace identities. To keep one Agents panel per task, tsk will generate a **task-level multi-root workspace** and open that instead:

```
~/tsk-tasks/<id>/task.code-workspace
```

Example shape:

```json
{
  "folders": [
    { "path": "workspace/my-api" },
    { "path": "workspace/my-web" }
  ],
  "settings": {}
}
```

Launch:

```bash
cursor ~/tsk-tasks/<id>/task.code-workspace
```

Cursor's Agents window filters conversations by workspace root; a multi-root `.code-workspace` file gives one root that spans every repo in the task. See [Cursor changelog — multi-root workspaces](https://cursor.com/changelog/04-24-26).

Until multi-repo task launch is implemented, continue opening Cursor from the task-specific repo path (`TSK_TASK_REPO`) as today — or via `tsk task editor` when using experimental Distrobox isolation.

## Related

- Daily use and task layout: [README.md](../README.md)
- Agent integration stubs: [notes/poc-plan.md](../notes/poc-plan.md) (Agent Integration Hooks)
- Cursor hooks for session metadata (future): [cursor.com/docs/hooks](https://cursor.com/docs/hooks)
