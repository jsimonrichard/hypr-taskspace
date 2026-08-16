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
    active_workspace_name: null
  })

  function refresh() {
    if (!statusProc.running) statusProc.running = true
  }

  function applyStatus(text) {
    try {
      const parsed = JSON.parse(text || "{}")
      if (parsed && typeof parsed === "object") root.status = parsed
    } catch (e) {
    }
  }

  function workspaceByName(name) {
    const values = Hyprland.workspaces.values
    for (let i = 0; i < values.length; i++) {
      if (values[i].name === name) return values[i]
    }
    return null
  }

  function visibleSlots() {
    const names = root.status.workspaces || []
    const visible = Math.max(1, Number(root.status.visible_workspace_count) || names.length)
    const active = Number(root.status.active_workspace) || 1
    const slots = []
    for (let i = 0; i < names.length; i++) {
      const index = i + 1
      if (index <= visible || index === active) slots.push(index)
    }
    return slots
  }

  function slotLabel(index) {
    const focusedName = Hyprland.focusedWorkspace ? Hyprland.focusedWorkspace.name : ""
    const name = (root.status.workspaces || [])[index - 1]
    if (name && focusedName === name) return "\uDB85\uDCFB"
    return index === 10 ? "0" : String(index)
  }

  function slotOccupied(index) {
    const name = (root.status.workspaces || [])[index - 1]
    const workspace = name ? root.workspaceByName(name) : null
    if (workspace) return workspace.toplevels.values.length > 0
    const occupied = root.status.occupied_workspace_indices || []
    return occupied.indexOf(index) !== -1
  }

  function slotFocused(index) {
    const name = (root.status.workspaces || [])[index - 1]
    if (!name || !Hyprland.focusedWorkspace) {
      return Number(root.status.active_workspace) === index
    }
    return Hyprland.focusedWorkspace.name === name
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

  Component.onCompleted: root.refresh()

  IpcHandler {
    target: "tsk.taskspace"

    function refresh(): void {
      root.broadcast("refresh")
    }
  }

  Process {
    id: statusProc
    command: [root.tskCmd, "bar", "status", "--json"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.applyStatus(text)
    }
  }

  FileView {
    path: (Quickshell.env("XDG_RUNTIME_DIR") || "/run/user/0") + "/tsk/state.rev"
    watchChanges: true
    printErrors: false
    onFileChanged: reload()
    onLoaded: root.refresh()
  }

  Timer {
    interval: 2000
    running: true
    repeat: true
    onTriggered: root.refresh()
  }

  GridLayout {
    id: grid
    anchors.fill: parent
    anchors.rightMargin: root.trailingGap
    columns: root.vertical ? 1 : (1 + root.visibleSlots().length)
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
      model: root.visibleSlots()

      WidgetButton {
        required property int modelData

        readonly property bool occupied: root.slotOccupied(modelData)
        readonly property bool focused: root.slotFocused(modelData)

        bar: root.bar
        text: root.slotLabel(modelData)
        opacity: occupied || focused ? 1 : 0.5
        horizontalMargin: 6
        verticalPadding: 6
        fixedWidth: root.vertical ? root.barSize : Style.space(20)
        fixedHeight: root.barSize
        onPressed: function() { root.switchWorkspace(modelData) }
      }
    }
  }
}
