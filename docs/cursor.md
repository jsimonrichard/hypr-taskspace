# Cursor integration

How tsk opens Cursor and how that will evolve when a task has multiple repos.

## Current behavior

When a task is created, `.tsk/on-start.sh` opens Cursor on the task's managed checkout:

```bash
cursor "$TSK_TASK_REPO"
```

`TSK_TASK_REPO` is the task-specific path under `~/tsk-tasks/<id>/workspace/<repo-folder-name>` (or `workspace/` for scratch tasks). Always use this path — not the canonical repo location elsewhere on disk — so Cursor scopes agent conversations to the task checkout.

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

Until multi-repo task launch is implemented, continue opening Cursor from the task-specific repo path (`TSK_TASK_REPO`) as today.

## Related

- Task home layout and `agent-notes.md`: [README.md](../README.md)
- Agent integration stubs: [notes/poc-plan.md](../notes/poc-plan.md) (Agent Integration Hooks)
- Cursor hooks for session metadata (future): [cursor.com/docs/hooks](https://cursor.com/docs/hooks)
