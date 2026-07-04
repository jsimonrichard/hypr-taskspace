# Task-Centric Agentic Coding UX Notes

> **Note:** The active implementation is the **Rust CLI** (`crates/tsk-cli`) and **Waybar CFFI module** (`crates/tsk-waybar`). See [README.md](../README.md). These notes describe the original product vision; Distrobox and window-routing pieces are deferred.

## Goal

Explore a middle-ground agentic coding UX between:

- cloud agents, which are portable and isolated but slow/expensive and awkward for non-web work
- local IDE agents, which are fast and accurate but become messy when multiple projects/tasks are active

The core idea is to build a **task-centric workspace system** that sits above the desktop/window manager and keeps task context clear across terminals, browsers, editors, logs, and containers.

---

## Main UX Problems We Want to Solve

### Problems with cloud agents
- Browser UI can be slow
- Non-web work often requires local cloning/building/testing anyway
- VM usage adds cost beyond LLM usage

### Problems with local IDE agents
- Multiple projects become hard to manage
- Many IDE windows / browser windows / terminals get confusing
- Worktrees are annoying to manage manually
- Hard to walk away, close the laptop, or switch devices
- Easy to accidentally mix unrelated changes in one codebase

---

## Core UX Direction

The likely right abstraction is:

**task > desktop/workspace > window**

Not just “project” or “desktop,” but a **task** as the top-level unit of work.

A task should own:
- a repo/worktree
- an environment/container
- browser profile
- terminal sessions
- agent state
- one or more desktops/workspaces
- window routing rules

The user should think:
- “I am in task A”
rather than:
- “I am on desktop 4”

---

## Why a Task Layer Above Desktops Makes Sense

A desktop/workspace is just a spatial bucket.

A task is semantic:
- same repo
- same branch
- same environment
- same browser profile
- same agent context
- same objective

If all windows from all tasks are equally accessible, the user gets the same selection problem again. So we need a strong notion of **current task context**.

Desired behavior:
- new windows created while in a task belong to that task by default
- switching windows should prefer windows within the current task
- switching tasks should change the active context
- task-local operations should be easier than global operations

---

## Proposed Mental Model

### 1. Task
Top-level unit of work.

Contains:
- environment
- repo/worktree
- agent session
- browser profile
- one or more desktops/workspaces
- status/metadata
- launched windows

### 2. Desktop / workspace group
A task may have multiple desktops/workspaces, such as:
- coding
- browser
- logs
- preview/testing

These are subordinate to the task.

### 3. Window
Concrete app surface that belongs to a task/workspace.

---

## Key UX Principle

**Make the task the thing that owns the surfaces.**

Windows are just manifestations of a task.

This gives:
- clean parallel work
- easier switching
- easier grouping
- less confusion about which terminal/browser/editor belongs where

---

## Recommended Direction for Isolation

Use containers for environment isolation, but keep the host desktop as the presentation layer.

A good model is:
- one task = one environment/container
- multiple windows can attach to that same environment
- the task owns the environment, not the window

This is better than trying to isolate per window.

---

## Distrobox vs BusyBox

### BusyBox
BusyBox is a compact bundle of Unix command-line utilities:
- `sh`
- `ls`
- `cp`
- `mount`
- etc.

It is useful in embedded systems and tiny images, but it is **not** a container manager.

### Distrobox
Distrobox is the relevant tool for this project.

It provides containerized Linux environments that integrate nicely with the host desktop/session.

It is typically built on top of:
- Podman
- Docker
- sometimes other container backends depending on platform/setup

So for this project:
- think **Distrobox** for task environments
- not BusyBox

---

## Container/Desktop Integration Options

If GUI apps run inside containers and appear on the host desktop, the common pattern is:
- share the host display/session sockets
- pass through DBus session access
- map UID/GID so file ownership works properly
- optionally pass audio and GPU access

This makes it possible to have:
- containerized terminals
- containerized dev tools
- real browser windows
- all integrated into one desktop session

### Relevant tools
- **Distrobox**: very good for integrated containerized dev environments
- **Toolbox**: similar idea, especially on Fedora-ish systems
- **systemd-nspawn**: more OS-like containers
- **LXC/LXD**: lightweight full-system containers
- **VMs**: best for fully isolated desktop sessions, but heavier

---

## Why Not Put the Browser Inside the IDE?

That approach is likely a dead end because:
- it does not preserve full browser devtools
- login/auth flows are awkward
- extensions can be limited
- browser-specific workflows are weaker than a real browser
- desktop apps are harder to integrate than simple editor panes

Better pattern:
- run a real browser
- give it a task-specific profile
- route it to the current task

---

## Why Separate Desktops Alone Are Not Enough

Separate desktops/workspaces can help, but they are not the full solution.

Problems:
- too coarse
- not semantically tied to the task
- may conflict with current workflows
- don’t solve browser profile separation
- don’t solve container/environment identity

So desktops should be treated as an implementation detail under a task abstraction.

---

## What the UX Should Feel Like

### When you enter a task
The system should:
- focus the task’s primary workspace
- mark the task as current
- launch new windows into that task by default
- set shell/editor/browser launch context

### When you open a new terminal/browser/editor
It should inherit the current task context automatically.

### When you switch windows
It should prefer windows within the current task first.

### When you switch tasks
The system should:
- update the current task indicator
- optionally switch to the task’s workspace set
- optionally restore the task’s surfaces

---

## Suggested Task State

A task should track:
- task ID
- task name
- repo/worktree
- container/environment
- browser profile
- active windows
- workspace IDs
- status
- agent notes
- logs/tests
- ports

A task is the source of truth for what belongs together.

---

## Ideas for Window Correlation

The challenge is correlating:
- terminal windows
- browser windows
- editor windows
- logs
- preview windows

Potential solutions:
- window title prefixes
- app class/instance naming
- workspace naming
- shell prompt markers
- browser profile names
- status bar indicators
- container/task names

Even simple labels can reduce cognitive load a lot.

---

## Good UX Pattern: Task Mode

There should be a clear visible notion of the **current task**, like:
- `Task: auth-fix`
- `Task: billing-refactor`
- `Task: none`

In task mode:
- new windows belong to that task
- switching windows is task-local by default
- launchers inherit that context
- the user always knows what they’re working on

---

## Good UX Pattern: Task-Specific Browser Profiles

For browser-heavy work, create one browser profile per task.

Benefits:
- separate logins
- separate cookies
- separate extensions
- separate devtools state
- less confusion about which browser belongs to which task

This is much better than embedding a browser into the IDE.

---

## Good UX Pattern: Task-Specific Worktrees

Each task should get:
- its own worktree or branch
- its own container
- its own browser profile
- its own workspace group

This helps avoid mixing unrelated changes in the same branch.

---

## Good UX Pattern: Task Launcher

Build a task-aware launcher that can:
- create a task
- switch task
- open terminal in task
- open browser in task
- open editor in task
- attach to existing task windows
- launch logs/dev servers in task context

This could be:
- a CLI
- a command palette
- a small local GUI
- a Hyprland-integrated launcher

---

## Hyprland as a PoC Platform

Hyprland looks like a strong place to prototype this because it is configurable and scriptable.

Useful pieces:
- `hyprctl`
- Hyprland IPC/events
- window rules
- class/title matching
- workspace management
- scriptable launchers

This makes it possible to experiment with:
- task routing
- window placement
- task indicators
- automatic new-window behavior

---

## Tools Worth Considering for the UX Side

### 1. Hyprland native hooks
Use:
- `hyprctl`
- window rules
- IPC/events
- title/class matching

For routing windows to tasks and workspaces.

### 2. Task daemon / control plane
A small local service that tracks:
- current task
- task metadata
- workspaces
- windows
- browser profiles
- containers
- ports

Could expose:
- CLI
- local HTTP API
- UNIX socket

### 3. Launcher / command palette
Possible tools:
- `fuzzel`
- `wofi`
- `rofi`
- `bemenu`

Use for:
- task switching
- new task creation
- window switching
- launching task-scoped apps

### 4. Status bar / indicator
Possible tools:
- `waybar`
- `eww`
- `ags`

Use to show:
- current task
- task status
- number of active tasks
- routing state

### 5. Terminal multiplexer integration
Possible tools:
- `tmux`
- `zellij`
- terminal-specific session tools

Useful for:
- task-aware shell sessions
- persistent command contexts
- per-task terminals

### 6. Browser profile management
Use real browser profiles, not embedded browsers.

### 7. Session restore / state persistence
Useful for:
- resuming tasks
- closing and reopening later
- switching devices
- recovering after restart

---

## Key UX Goals

- Task-local behavior should be the default
- Global access should be explicit, not accidental
- Switching tasks should be easy
- Switching windows within a task should be easier than switching across tasks
- New windows should inherit the current task automatically
- The user should always have a clear indicator of the current task
- Multiple tasks should remain cleanly separated even if they share the same desktop session

---

## Suggested Architecture for a PoC

### Phase 1
Build a task-aware launcher/CLI:
- `task start`
- `task switch`
- `task terminal`
- `task browser`
- `task editor`

### Phase 2
Add visible state:
- task label in status bar
- window title prefixes
- workspace naming

### Phase 3
Add routing:
- new windows go to the current task by default
- windows are moved automatically into task workspaces

### Phase 4
Add a task switcher:
- fuzzy list of tasks
- show windows per task
- jump to the task
- restore task surfaces

---

## Practical Recommendation

Start with:
- **Hyprland**
- **Distrobox**
- **task-scoped environments**
- **task-scoped browser profiles**
- **task-aware launcher/control plane**
- **workspace/window routing via Hyprland IPC**

Keep the WM as the base system, and build the task layer above it.

---

## Summary

The likely right abstraction is:

**task > workspace/desktop > window**

And the likely right implementation shape is:

- one task = one isolated environment/container
- multiple windows can belong to the same task
- task context determines where new windows go
- the desktop is a routing/rendering layer, not the primary unit of work
- a local task control plane manages state and switching

This preserves:
- isolation
- clean parallel work
- native browser/editor/terminal experiences
- less cognitive overload
- easier switching and resumption
