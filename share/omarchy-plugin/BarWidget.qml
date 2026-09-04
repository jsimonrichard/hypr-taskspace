import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Hyprland
import Quickshell.Io
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "tsk.taskspace"

  readonly property string tskCmd: "@TSK_CMD@"
  property var status: ({
    task_id: null,
    task_name: null,
    repo_name: null,
    workspaces: [],
    visible_workspace_count: 5,
    occupied_workspace_indices: [],
    active_workspace: 1,
    active_workspace_name: null,
    global_workspace_slots: []
  })
  // Hoisted so Repeater delegates bind to a real QML property, not a nested
  // JSON field (those don't always retrigger).
  property var globalWorkspaceSlots: []
  property var occupiedWorkspaceIndices: []
  property bool refreshQueued: false

  // Subscribe to each workspace's lastIpcObject.windows so occupancy
  // re-evaluates when Hyprland fills in window counts after a plugin reload
  // (workspaces.values.length often stays the same).
  readonly property int liveWindowsRev: {
    const _ = root.hyprRev
    const values = Hyprland.workspaces ? Hyprland.workspaces.values : []
    let n = 0
    for (let i = 0; i < values.length; i++) {
      const ipc = values[i].lastIpcObject
      if (ipc && Number(ipc.windows) > 0) n += Number(ipc.windows)
      const tops = values[i].toplevels ? values[i].toplevels.values : null
      if (tops) n += tops.length
    }
    return n
  }

  // Match Waybar `#cffi-tsk #tsk-workspaces label.global` (`share/waybar/tsk-style.css`).
  readonly property color globalSlotColor: "#7ab392"
  readonly property color globalSlotEmptyColor: Qt.rgba(122 / 255, 179 / 255, 146 / 255, 0.55)

  function refresh() {
    if (statusProc.running) {
      root.refreshQueued = true
      return
    }
    statusProc.running = true
  }

  function applyStatus(text) {
    try {
      const parsed = JSON.parse(text || "{}")
      if (!parsed || typeof parsed !== "object") return
      root.status = parsed
      const raw = parsed.global_workspace_slots
      const next = []
      if (raw && raw.length) {
        for (let i = 0; i < raw.length; i++) next.push(Number(raw[i]))
      }
      root.globalWorkspaceSlots = next
      const occ = parsed.occupied_workspace_indices
      const nextOcc = []
      if (occ && occ.length) {
        for (let i = 0; i < occ.length; i++) nextOcc.push(Number(occ[i]))
      }
      root.occupiedWorkspaceIndices = nextOcc
    } catch (e) {
    }
  }

  // Bumped on Hyprland events so occupancy/focus bindings re-run even when
  // nested `toplevels` lists don't notify on their own.
  property int hyprRev: 0
  readonly property int minVisibleSlots: 5
  readonly property string focusedWorkspaceName: Hyprland.focusedWorkspace ? String(Hyprland.focusedWorkspace.name) : ""

  function workspaceByName(name) {
    const values = Hyprland.workspaces.values
    const want = String(name)
    for (let i = 0; i < values.length; i++) {
      if (String(values[i].name) === want) return values[i]
    }
    return null
  }

  // Nested `workspace.toplevels` often fails to notify, and a workspace object
  // with an empty toplevel list used to win over `tsk bar status` occupancy
  // (false empty). Treat a slot as occupied if ANY live source says so.
  function workspaceOccupiedLive(name) {
    if (!name) return false
    const want = String(name)
    const values = Hyprland.workspaces ? Hyprland.workspaces.values : null
    if (!values) return false
    for (let i = 0; i < values.length; i++) {
      const ws = values[i]
      if (String(ws.name) !== want) continue
      const ipc = ws.lastIpcObject
      if (ipc && Number(ipc.windows) > 0) return true
      const tops = ws.toplevels ? ws.toplevels.values : null
      if (tops && tops.length > 0) return true
    }
    return false
  }

  readonly property int focusedSlotIndex: {
    const names = root.status.workspaces || []
    const focusedName = root.focusedWorkspaceName
    if (focusedName) {
      for (let i = 0; i < names.length; i++) {
        if (names[i] === focusedName) return i + 1
      }
    }
    return Number(root.status.active_workspace) || 1
  }

  readonly property int highestOccupiedSlot: {
    const _ = root.hyprRev
    const __ = root.liveWindowsRev
    const ___ = root.occupiedWorkspaceIndices
    const names = root.status.workspaces || []
    let highest = 0
    for (let i = 0; i < names.length; i++) {
      if (root.slotOccupied(i + 1)) highest = i + 1
    }
    return highest
  }

  // Live copy of Waybar `visible_default_workspace_count`: min 5, expand to
  // the focused or highest occupied slot. Do not use JSON `visible_workspace_count`
  // here — that lags on `tsk bar status` / the 2s poll.
  readonly property var slotModel: {
    const names = root.status.workspaces || []
    const total = names.length
    const visible = Math.min(
      Math.max(root.minVisibleSlots, root.focusedSlotIndex, root.highestOccupiedSlot),
      total,
      10
    )
    const slots = []
    for (let i = 1; i <= visible; i++) slots.push(i)
    return slots
  }

  function slotLabel(index) {
    const name = (root.status.workspaces || [])[index - 1]
    if (name && root.focusedWorkspaceName === name) return "\uDB85\uDCFB"
    return index === 10 ? "0" : String(index)
  }

  function slotOccupied(index) {
    const _ = root.hyprRev
    const __ = root.liveWindowsRev
    const n = Number(index)
    const occupied = root.occupiedWorkspaceIndices
    for (let i = 0; i < occupied.length; i++) {
      if (Number(occupied[i]) === n) return true
    }
    const name = (root.status.workspaces || [])[index - 1]
    return root.workspaceOccupiedLive(name)
  }

  function slotFocused(index) {
    const name = (root.status.workspaces || [])[index - 1]
    if (!name || !root.focusedWorkspaceName) {
      return Number(root.status.active_workspace) === index
    }
    return root.focusedWorkspaceName === name
  }

  function slotGlobal(index) {
    const slots = root.globalWorkspaceSlots
    const n = Number(index)
    for (let i = 0; i < slots.length; i++) {
      if (Number(slots[i]) === n) return true
    }
    // Same rule as Waybar: a purely numeric workspace name in the global set.
    const name = String((root.status.workspaces || [])[index - 1] || "")
    if (!/^[0-9]+$/.test(name)) return false
    const named = Number(name)
    for (let i = 0; i < slots.length; i++) {
      if (Number(slots[i]) === named) return true
    }
    return false
  }

  function taskLabel() {
    const name = root.status.task_name
    if (!name) return "󰣇 default"
    if (root.status.repo_name) return "󱓝 " + root.status.repo_name + ": " + name
    return "󱓝 " + name
  }

  function openTaskTui() {
    if (!root.bar) return
    root.bar.run(Util.shellQuote(root.tskCmd) + " task tui-launch")
  }

  function switchWorkspace(index) {
    if (!root.bar) return
    root.bar.run(Util.shellQuote(root.tskCmd) + " workspace switch " + index)
  }

  readonly property real trailingGap: root.vertical ? 0 : Style.spaceReal(1.5)

  implicitWidth: grid.implicitWidth + trailingGap
  implicitHeight: grid.implicitHeight

  function pingHyprland() {
    if (Hyprland.refreshWorkspaces) Hyprland.refreshWorkspaces()
    root.hyprRev++
  }

  function statusEnvironment() {
    const env = ({ TSK_CMD: root.tskCmd })
    const sig = Quickshell.env("HYPRLAND_INSTANCE_SIGNATURE")
    if (sig) env.HYPRLAND_INSTANCE_SIGNATURE = sig
    const runtime = Quickshell.env("XDG_RUNTIME_DIR")
    if (runtime) env.XDG_RUNTIME_DIR = runtime
    return env
  }

  Component.onCompleted: {
    root.pingHyprland()
    root.refresh()
  }

  IpcHandler {
    target: "tsk.taskspace"

    function refresh(): void {
      root.broadcast("refresh")
    }
  }

  Process {
    id: statusProc
    // Additive: do not set HYPRLAND to "" or it wipes an inherited value.
    environment: root.statusEnvironment()
    command: ["sh", "-c",
      "if [ -z \"$HYPRLAND_INSTANCE_SIGNATURE\" ]; then " +
      "for d in \"$XDG_RUNTIME_DIR/hypr\"/*; do " +
      "[ -S \"$d/.socket.sock\" ] || continue; " +
      "export HYPRLAND_INSTANCE_SIGNATURE=\"${d##*/}\"; break; " +
      "done; fi; " +
      "exec \"$TSK_CMD\" bar status --json"
    ]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        root.applyStatus(text)
        if (root.refreshQueued) {
          root.refreshQueued = false
          statusProc.running = true
        }
      }
    }
  }

  FileView {
    path: (Quickshell.env("XDG_RUNTIME_DIR") || "/run/user/0") + "/tsk/state.rev"
    watchChanges: true
    printErrors: false
    onFileChanged: reload()
    onLoaded: root.refresh()
  }

  Connections {
    target: Hyprland
    function onRawEvent(event) {
      if (!event || !event.name) return
      const name = String(event.name)
      if (name === "workspace" || name === "workspacev2"
          || name === "focusedmon" || name === "focusedmonv2"
          || name === "renameworkspace") {
        root.hyprRev++
        return
      }
      if (name === "openwindow" || name === "closewindow"
          || name === "movewindow" || name === "movewindowv2"
          || name === "createworkspace" || name === "createworkspacev2"
          || name === "destroyworkspace" || name === "destroyworkspacev2") {
        root.pingHyprland()
        root.refresh()
      }
    }
  }

  Timer {
    interval: 2000
    running: true
    repeat: true
    onTriggered: {
      root.pingHyprland()
      root.refresh()
    }
  }

  GridLayout {
    id: grid
    anchors.fill: parent
    anchors.rightMargin: root.trailingGap
    columns: root.vertical ? 1 : (1 + root.slotModel.length)
    columnSpacing: root.vertical ? 0 : Style.space(1)
    rowSpacing: root.vertical ? Style.space(2) : 0

    WidgetButton {
      bar: root.bar
      text: root.taskLabel()
      horizontalMargin: 8
      verticalPadding: 6
      fixedHeight: root.barSize
      onPressed: function() { root.openTaskTui() }
    }

    Repeater {
      model: root.slotModel

      WidgetButton {
        required property int modelData

        readonly property bool occupied: {
          const _ = root.occupiedWorkspaceIndices
          return root.slotOccupied(modelData)
        }
        readonly property bool focused: root.slotFocused(modelData)
        readonly property bool isGlobal: root.slotGlobal(modelData)

        bar: root.bar
        text: root.slotLabel(modelData)
        // WidgetButton.foreground tracks the bar theme. Custom colors go
        // through active/activeColor (same path as urgent indicators).
        active: isGlobal
        useActiveColor: isGlobal
        activeColor: occupied || focused ? root.globalSlotColor : root.globalSlotEmptyColor
        opacity: occupied || focused || isGlobal ? 1 : 0.5
        tooltipText: isGlobal ? (String(modelData) + " (global)") : ""
        horizontalMargin: 6
        verticalPadding: 6
        fixedWidth: root.vertical ? root.barSize : Style.space(20)
        fixedHeight: root.barSize
        onPressed: function() { root.switchWorkspace(modelData) }
      }
    }
  }
}
