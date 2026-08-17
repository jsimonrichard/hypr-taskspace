import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import QtQuick
import qs.Commons
import qs.Ui
import "TaskspaceModel.js" as Model

Item {
  id: root

  property string omarchyPath: Quickshell.env("OMARCHY_PATH")
  property var shell: null
  property var manifest: null

  readonly property string tskCmd: "@TSK_CMD@"
  property bool opened: false
  property string tab: "tasks"
  property string screen: "list"
  property string filterText: ""
  property int selectedIndex: 0
  property bool cursorActive: false
  property var activeTasks: []
  property var archivedTasks: []
  property var repos: []
  property string listError: ""
  property string formError: ""
  property string formName: ""
  property string formFocus: "name"
  property int formRepoIndex: 0
  property bool formWorktree: true
  property bool formContainer: false
  property string renameName: ""
  property string pendingTab: ""
  property string pendingError: ""
  property bool pendingReopen: false
  property string progressLog: ""
  property string progressTaskId: ""
  property bool progressDone: false
  property bool progressFailed: false
  property string confirmAction: ""
  property string confirmId: ""
  property string confirmLabel: ""
  property bool selectCurrentOnRebuild: false

  property color background: Color.menu.background
  property color foreground: Color.menu.text
  property color border: Color.menu.border
  property var borderSpec: Border.surfaceSpec("menu", "border", border, Math.max(1, Style.space(2)))
  property color scrim: Color.menu.scrim
  property color selectedBackground: Color.menu.selectedBackground
  property color selectedText: Color.menu.selectedText
  readonly property int cornerRadius: Style.cornerRadius
  property string fontFamily: Style.font.menuFamily
  property int contentMargin: Style.spacing.panelPadding
  property int headerHeight: Math.max(Style.space(34), Style.font.title + Style.spacing.controlPaddingY * 2)
  property int tabHeight: Math.max(Style.space(28), Style.font.body + Style.spacing.controlPaddingY)
  property int footerHeight: Math.max(Style.space(22), Style.font.caption + Style.spacing.xs)
  property int contentSpacing: Style.spacing.md
  property int cardWidth: Math.min(Style.space(480), panel.width - Style.gapsOut * 2)
  property int cardHeight: Math.min(Style.space(560), panel.height - Style.gapsOut * 2)
  property int rowHeight: Math.max(Style.space(50), Style.font.body + Style.font.caption + Style.spacing.rowPaddingX * 2)

  readonly property var formRepoChoices: {
    const scratch = [{ id: "scratch", name: "No repo (scratch workspace)", path: "", kind: "scratch" }]
    const listed = (root.repos || []).map(function(repo) {
      return { id: repo.id, name: repo.name, path: repo.path, kind: "repo" }
    })
    return scratch.concat(listed)
  }

  readonly property var formRepo: {
    const items = root.formRepoChoices
    if (root.formRepoIndex < 0 || root.formRepoIndex >= items.length) return items[0]
    return items[root.formRepoIndex]
  }

  function open(payloadJson) {
    let tab = root.pendingTab
    try {
      const payload = JSON.parse(payloadJson || "{}")
      if (payload && payload.tab) tab = payload.tab
    } catch (e) {
    }
    root.opened = true
    root.tab = tab || "tasks"
    root.pendingTab = ""
    root.screen = "list"
    root.filterText = ""
    root.selectedIndex = 0
    root.cursorActive = true
    root.selectCurrentOnRebuild = root.tab === "tasks"
    root.formError = root.pendingError
    root.pendingError = ""
    root.disarmPointer()
    root.reloadAll()
    Qt.callLater(function() { keyCatcher.forceActiveFocus() })
  }

  function close() {
    root.opened = false
  }

  function dismiss() {
    root.opened = false
    if (root.shell && typeof root.shell.hide === "function")
      root.shell.hide((root.manifest && root.manifest.id) || "tsk.taskspace")
  }

  function toggle() {
    if (root.opened) root.dismiss()
    else root.open("{}")
  }

  function refresh() {
    root.reloadAll()
    return "ok"
  }

  function reloadAll() {
    root.reloadTasks()
    root.reloadRepos()
  }

  function reloadTasks() {
    if (!listProc.running) listProc.running = true
  }

  function reloadRepos() {
    if (!repoProc.running) repoProc.running = true
  }

  function applyList(text, exitCode) {
    if (Number(exitCode) !== 0) {
      root.listError = "tskd is not running"
      root.activeTasks = []
      root.archivedTasks = []
      root.rebuildDisplay()
      return
    }
    try {
      const parsed = Model.parseTaskList(text)
      root.activeTasks = parsed.active
      root.archivedTasks = parsed.archived
      root.listError = ""
    } catch (e) {
      root.listError = "could not read tasks"
      root.activeTasks = []
      root.archivedTasks = []
    }
    root.rebuildDisplay()
  }

  function applyRepos(text, exitCode) {
    if (Number(exitCode) !== 0) {
      root.repos = []
      if (root.tab === "repos") root.rebuildDisplay()
      return
    }
    try {
      root.repos = Model.parseRepos(text)
    } catch (e) {
      root.repos = []
    }
    if (root.tab === "repos" || root.screen === "new") root.rebuildDisplay()
  }

  function sourceItems() {
    if (root.tab === "repos") {
      return (root.repos || []).map(function(repo) {
        return { id: repo.id, name: repo.name, path: repo.path, kind: "repo", status: "", repo_name: "", current: false }
      })
    }
    if (root.tab === "archived") {
      return (root.archivedTasks || []).map(function(item) {
        return {
          id: item.id,
          name: item.name,
          kind: "task",
          status: item.status || "archived",
          repo_name: item.repo_name || "",
          listed_at: item.listed_at || "",
          current: false
        }
      })
    }
    return root.activeTasks || []
  }

  function filteredItems() {
    const query = String(root.filterText || "").trim().toLowerCase()
    return Model.sortItems(root.sourceItems().filter(function(item) { return Model.matches(item, query) }))
  }

  function rebuildDisplay() {
    const items = root.filteredItems()
    displayModel.clear()
    for (let i = 0; i < items.length; i++) {
      const item = items[i]
      displayModel.append({
        taskId: String(item.id || ""),
        name: String(item.name || ""),
        kind: String(item.kind || "task"),
        status: String(item.status || ""),
        repo_name: String(item.repo_name || ""),
        current: item.current === true,
        path: String(item.path || ""),
        label: Model.rowLabel(item),
        icon: Model.rowIcon(item),
        detail: Model.rowDetail(item)
      })
    }
    if ((root.tab === "tasks" || root.tab === "repos") && !root.listError) {
      const action = root.tab === "repos"
        ? { kind: "new-repo", name: "New Repo" }
        : { kind: "new-task", name: "New Task" }
      displayModel.append({
        taskId: "",
        name: action.name,
        kind: action.kind,
        status: "",
        repo_name: "",
        current: false,
        path: "",
        label: Model.rowLabel(action),
        icon: Model.rowIcon(action),
        detail: Model.rowDetail(action)
      })
    }
    if (root.selectCurrentOnRebuild && root.tab === "tasks") {
      root.selectCurrentOnRebuild = false
      root.selectedIndex = root.indexOfCurrent()
    } else if (displayModel.count === 0) {
      root.selectedIndex = 0
    } else if (root.selectedIndex >= displayModel.count) {
      root.selectedIndex = displayModel.count - 1
    } else if (root.selectedIndex < 0) {
      root.selectedIndex = 0
    }
    root.cursorActive = displayModel.count > 0
    Qt.callLater(function() {
      if (displayModel.count > 0 && resultList.visible)
        resultList.positionViewAtIndex(root.selectedIndex, ListView.Contain)
    })
  }

  function indexOfCurrent() {
    for (let i = 0; i < displayModel.count; i++) {
      if (displayModel.get(i).current === true) return i
    }
    return 0
  }

  function selectedRow() {
    if (root.selectedIndex < 0 || root.selectedIndex >= displayModel.count) return null
    return displayModel.get(root.selectedIndex)
  }

  function cycleTab(delta) {
    const tabs = ["tasks", "archived", "repos"]
    const index = Math.max(0, tabs.indexOf(root.tab))
    root.tab = tabs[(index + delta + tabs.length) % tabs.length]
    root.filterText = ""
    root.selectedIndex = 0
    root.selectCurrentOnRebuild = root.tab === "tasks"
    root.rebuildDisplay()
  }

  function showList() {
    root.screen = "list"
    root.formError = ""
    root.rebuildDisplay()
    Qt.callLater(function() { keyCatcher.forceActiveFocus() })
  }

  function beginCreate() {
    if (root.tab === "repos") {
      root.pickRepoDirectory()
      return
    }
    root.screen = "new"
    root.formName = ""
    root.formFocus = "name"
    root.formRepoIndex = 0
    root.formWorktree = true
    root.formContainer = false
    root.formError = ""
    root.reloadRepos()
  }

  function beginRename() {
    const row = root.selectedRow()
    if (!row || row.kind === "default" || row.kind === "repo" || row.kind === "new-task" || row.kind === "new-repo") return
    root.screen = "rename"
    root.renameName = row.label
    root.formError = ""
  }

  function requestArchive() {
    const row = root.selectedRow()
    if (!row || row.kind !== "task" || row.status === "archived") return
    root.openConfirm("archive", row.taskId, row.label, "Archive “" + row.label + "”?")
  }

  function requestRestore() {
    const row = root.selectedRow()
    if (!row || row.status !== "archived") return
    root.openConfirm("restore", row.taskId, row.label, "Restore “" + row.label + "”?")
  }

  function requestDelete() {
    const row = root.selectedRow()
    if (!row || row.kind === "default" || row.kind === "new-task" || row.kind === "new-repo") return
    if (row.kind === "repo") {
      root.openConfirm("remove-repo", row.taskId, row.label, "Unregister “" + row.label + "”? The checkout stays on disk.")
      return
    }
    root.openConfirm("delete", row.taskId, row.label, "Delete “" + row.label + "”? This cannot be undone.")
  }

  function openConfirm(action, id, label, message) {
    root.confirmAction = action
    root.confirmId = id
    root.confirmLabel = label
    confirmDialog.message = message
    confirmDialog.confirmText = action === "delete" || action === "remove-repo" ? "Delete" : "Confirm"
    confirmDialog.selectedIndex = 1
    confirmDialog.opened = true
  }

  function runTsk(args) {
    actionProc.command = [root.tskCmd].concat(args)
    actionProc.running = true
  }

  function activateIndex(index) {
    if (index < 0 || index >= displayModel.count) return
    const row = displayModel.get(index)
    if (!row) return
    if (root.tab === "archived") {
      root.openConfirm("restore", row.taskId, row.label, "Restore “" + row.label + "”?")
      return
    }
    if (row.kind === "new-task" || row.kind === "new-repo") {
      root.beginCreate()
      return
    }
    if (root.tab === "repos") return
    root.dismiss()
    if (row.kind === "default")
      Util.execDetached(Util.shellQuote(root.tskCmd) + " taskspace default")
    else
      Util.execDetached(Util.shellQuote(root.tskCmd) + " task switch " + Util.shellQuote(row.taskId))
  }

  function submitNew() {
    const name = String(root.formName || "").trim()
    if (!name) {
      root.formError = "Name is required"
      root.formFocus = "name"
      return
    }
    const args = ["task", "new", name]
    const repo = root.formRepo
    if (!repo || repo.kind === "scratch") args.push("--scratch")
    else {
      args.push("--repo-path", repo.path)
      if (!root.formWorktree) args.push("--no-worktree")
    }
    if (root.formContainer) {
      args.push("--container")
      root.screen = "progress"
      root.progressLog = "Creating “" + name + "”…\n"
      root.progressTaskId = ""
      root.progressDone = false
      root.progressFailed = false
      createProc.command = [root.tskCmd].concat(args)
      createProc.running = true
      return
    }
    root.runTsk(args)
    root.pendingClose = true
  }

  function submitRename() {
    const row = root.selectedRow()
    const name = String(root.renameName || "").trim()
    if (!row || !name) {
      root.formError = "Name is required"
      return
    }
    root.runTsk(["task", "rename", row.taskId, name])
    root.showList()
  }

  function pickRepoDirectory() {
    if (folderPickProc.running) return
    root.pendingTab = "repos"
    root.pendingError = ""
    root.pendingReopen = false
    root.dismiss()
    folderPickProc.running = true
  }

  function reopenOverlay() {
    const id = (root.manifest && root.manifest.id) || "tsk.taskspace"
    if (root.shell && typeof root.shell.summon === "function")
      root.shell.summon(id, "{}")
    else
      root.open("{}")
  }

  function confirmPending() {
    const action = root.confirmAction
    const id = root.confirmId
    confirmDialog.opened = false
    if (action === "archive") root.runTsk(["task", "archive", id])
    else if (action === "restore") root.runTsk(["task", "restore", id])
    else if (action === "delete") root.runTsk(["task", "delete", id])
    else if (action === "remove-repo") root.runTsk(["repo", "remove", id])
  }

  function cycleFormFocus(delta) {
    let fields = ["name", "repo"]
    if (root.formRepo && root.formRepo.kind === "repo") fields.push("worktree")
    fields.push("container")
    const index = Math.max(0, fields.indexOf(root.formFocus))
    root.formFocus = fields[(index + delta + fields.length) % fields.length]
  }

  function appendFilter(target, event) {
    if (Util.editsFilter(event, target)) return Util.editedFilter(event, target)
    if (event.text && event.text.length === 1 && event.text.charCodeAt(0) >= 32 && event.text.charCodeAt(0) !== 127)
      return target + event.text
    return null
  }

  function select(delta) {
    if (displayModel.count === 0) return
    if (!root.cursorActive) {
      root.cursorActive = true
      root.selectedIndex = delta < 0 ? displayModel.count - 1 : 0
    } else {
      root.selectedIndex = (root.selectedIndex + delta + displayModel.count) % displayModel.count
    }
    resultList.positionViewAtIndex(root.selectedIndex, ListView.Contain)
  }

  function selectAbsolute(index) {
    if (displayModel.count === 0) return
    root.cursorActive = true
    root.selectedIndex = Math.max(0, Math.min(index, displayModel.count - 1))
    resultList.positionViewAtIndex(root.selectedIndex, ListView.Contain)
  }

  function setFilter(nextFilter) {
    root.filterText = nextFilter
    root.selectedIndex = 0
    root.cursorActive = true
    root.disarmPointer()
    root.rebuildDisplay()
  }

  function disarmPointer() {
    pointerGate.reset()
  }

  function selectFromPointer(index, item, mouse) {
    if (!pointerGate.moved(item, mouse)) return
    root.cursorActive = true
    root.selectedIndex = index
  }

  function handleListKey(event) {
    if (event.key === Qt.Key_Escape) {
      if (root.filterText) root.setFilter("")
      else root.dismiss()
      return true
    }
    if (event.key === Qt.Key_Tab) {
      root.cycleTab(event.modifiers & Qt.ShiftModifier ? -1 : 1)
      return true
    }
    if (event.key === Qt.Key_Left) {
      root.cycleTab(-1)
      return true
    }
    if (event.key === Qt.Key_Right) {
      root.cycleTab(1)
      return true
    }
    if (event.modifiers & Qt.AltModifier) {
      if (event.key === Qt.Key_N) { root.beginCreate(); return true }
      if (event.key === Qt.Key_E) { root.beginRename(); return true }
      if (event.key === Qt.Key_R) { root.requestRestore(); return true }
      if (event.key === Qt.Key_D && (event.modifiers & Qt.ShiftModifier)) { root.requestDelete(); return true }
      if (event.key === Qt.Key_D) {
        if (root.tab === "repos") root.requestDelete()
        else root.requestArchive()
        return true
      }
      return false
    }
    if (event.key === Qt.Key_Up) { root.select(-1); return true }
    if (event.key === Qt.Key_Down) { root.select(1); return true }
    if (event.key === Qt.Key_PageUp) { root.select(-6); return true }
    if (event.key === Qt.Key_PageDown) { root.select(6); return true }
    if (event.key === Qt.Key_Home) { root.selectAbsolute(0); return true }
    if (event.key === Qt.Key_End) { root.selectAbsolute(displayModel.count - 1); return true }
    if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
      if (root.cursorActive) root.activateIndex(root.selectedIndex)
      else if (displayModel.count > 0) root.cursorActive = true
      return true
    }
    if (Util.editsFilter(event, root.filterText)) {
      root.setFilter(Util.editedFilter(event, root.filterText))
      return true
    }
    if (event.text && event.text.length === 1 && event.text.charCodeAt(0) >= 32 && event.text.charCodeAt(0) !== 127) {
      root.setFilter(root.filterText + event.text)
      return true
    }
    return false
  }

  function handleNewKey(event) {
    if (event.key === Qt.Key_Escape) { root.showList(); return true }
    if (event.key === Qt.Key_Tab) { root.cycleFormFocus(event.modifiers & Qt.ShiftModifier ? -1 : 1); return true }
    if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) { root.submitNew(); return true }
    if (root.formFocus === "repo" && (event.key === Qt.Key_Up || event.key === Qt.Key_Down)) {
      const count = root.formRepoChoices.length
      if (count === 0) return true
      root.formRepoIndex = (root.formRepoIndex + (event.key === Qt.Key_Up ? -1 : 1) + count) % count
      return true
    }
    if ((root.formFocus === "worktree" || root.formFocus === "container") && event.key === Qt.Key_Space) {
      if (root.formFocus === "worktree") root.formWorktree = !root.formWorktree
      else root.formContainer = !root.formContainer
      return true
    }
    if (root.formFocus === "name") {
      const next = root.appendFilter(root.formName, event)
      if (next !== null) { root.formName = next; root.formError = ""; return true }
    }
    return false
  }

  function handleRenameKey(event) {
    if (event.key === Qt.Key_Escape) { root.showList(); return true }
    if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) { root.submitRename(); return true }
    const next = root.appendFilter(root.renameName, event)
    if (next !== null) { root.renameName = next; root.formError = ""; return true }
    return false
  }

  function handleProgressKey(event) {
    if (!root.progressDone) return true
    if (event.key === Qt.Key_Escape || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
      if (root.progressFailed) root.showList()
      else root.dismiss()
      return true
    }
    return true
  }

  property bool pendingClose: false

  ListModel { id: displayModel }

  PointerMoveGate {
    id: pointerGate
    referenceItem: card
  }

  Process {
    id: listProc
    command: [root.tskCmd, "task", "list", "--json", "--archived"]
    stdout: StdioCollector {
      id: listOut
      waitForEnd: true
    }
    onExited: function(exitCode) { root.applyList(listOut.text, exitCode) }
  }

  Process {
    id: repoProc
    command: [root.tskCmd, "repo", "list"]
    stdout: StdioCollector {
      id: repoOut
      waitForEnd: true
    }
    onExited: function(exitCode) { root.applyRepos(repoOut.text, exitCode) }
  }

  Process {
    id: actionProc
    stdout: StdioCollector { waitForEnd: true }
    onExited: function(exitCode) {
      if (root.pendingClose) {
        root.pendingClose = false
        if (Number(exitCode) === 0) root.dismiss()
        else {
          root.formError = "Command failed"
          root.screen = "new"
        }
        return
      }
      if (root.pendingReopen) {
        root.pendingReopen = false
        if (Number(exitCode) !== 0)
          root.pendingError = "Could not register that folder — pick a git or jj checkout"
        root.reopenOverlay()
        return
      }
      root.reloadAll()
    }
  }

  Process {
    id: folderPickProc
    command: [
      (root.omarchyPath || "/usr/share/omarchy") + "/bin/omarchy-file-select",
      "--directory",
      "--title",
      "Register repo"
    ]
    stdout: StdioCollector {
      id: folderPickOut
      waitForEnd: true
    }
    onExited: function(exitCode) {
      const path = String(folderPickOut.text || "").trim().split("\n")[0]
      if (Number(exitCode) !== 0 || !path) {
        root.reopenOverlay()
        return
      }
      root.pendingReopen = true
      root.runTsk(["repo", "add", path])
    }
  }

  Process {
    id: createProc
    stdout: SplitParser {
      onRead: function(line) {
        root.progressLog += line + "\n"
        if (!root.progressTaskId) root.progressTaskId = Model.createdTaskId(root.progressLog)
      }
    }
    onExited: function(exitCode) {
      root.progressDone = true
      root.progressFailed = Number(exitCode) !== 0
      if (root.progressFailed) root.progressLog += "\nFailed.\n"
      else root.progressLog += "\nDone. Press Enter to close.\n"
    }
  }

  FileView {
    path: (Quickshell.env("XDG_RUNTIME_DIR") || "/run/user/0") + "/tsk/state.rev"
    watchChanges: true
    printErrors: false
    onFileChanged: reload()
    onLoaded: if (root.opened && root.screen === "list") root.reloadAll()
  }

  PanelWindow {
    id: panel
    visible: root.opened
    anchors { top: true; bottom: true; left: true; right: true }
    color: "transparent"
    WlrLayershell.namespace: "tsk-taskspace"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive
    exclusionMode: ExclusionMode.Ignore

    Rectangle {
      anchors.fill: parent
      color: root.scrim
    }

    MouseArea {
      anchors.fill: parent
      onClicked: root.dismiss()
    }

    BorderSurface {
      id: card
      width: root.cardWidth
      height: root.cardHeight
      radius: root.cornerRadius
      anchors.centerIn: parent
      color: root.background
      borderSpec: root.borderSpec
      padding: root.contentMargin

      MouseArea { anchors.fill: parent; onClicked: {} }

      Item {
        id: keyCatcher
        anchors.fill: parent
        z: confirmDialog.opened ? 20 : 0
        focus: true

        Keys.priority: Keys.BeforeItem
        Keys.onPressed: function(event) {
          if (confirmDialog.opened) {
            if (confirmDialog.handleKey(event)) event.accepted = true
            return
          }
          let handled = false
          if (root.screen === "list") handled = root.handleListKey(event)
          else if (root.screen === "new") handled = root.handleNewKey(event)
          else if (root.screen === "rename") handled = root.handleRenameKey(event)
          else if (root.screen === "progress") handled = root.handleProgressKey(event)
          if (handled) event.accepted = true
        }

        ConfirmDialog {
          id: confirmDialog
          anchors.fill: parent
          opened: false
          z: 10
          background: root.background
          foreground: root.foreground
          scrim: root.scrim
          selectedBackground: root.selectedBackground
          selectedText: root.selectedText
          fontFamily: root.fontFamily
          cornerRadius: root.cornerRadius
          onCanceled: confirmDialog.opened = false
          onConfirmed: root.confirmPending()
        }
      }

      Column {
        anchors.fill: parent
        anchors.topMargin: card.contentTopInset
        anchors.rightMargin: card.contentRightInset
        anchors.bottomMargin: card.contentBottomInset
        anchors.leftMargin: card.contentLeftInset
        spacing: root.contentSpacing

        Row {
          width: parent.width
          height: root.tabHeight
          spacing: Style.space(8)
          visible: root.screen === "list"

          Repeater {
            model: [
              { id: "tasks", label: "Tasks" },
              { id: "archived", label: "Archived" },
              { id: "repos", label: "Repos" }
            ]

            Rectangle {
              required property var modelData
              width: tabLabel.implicitWidth + Style.space(16)
              height: parent.height
              radius: root.cornerRadius
              color: root.tab === modelData.id ? root.selectedBackground : "transparent"

              Text {
                id: tabLabel
                anchors.centerIn: parent
                text: parent.modelData.label
                color: root.tab === parent.modelData.id ? root.selectedText : root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.body
              }

              MouseArea {
                anchors.fill: parent
                cursorShape: Qt.PointingHandCursor
                onClicked: {
                  root.tab = parent.modelData.id
                  root.filterText = ""
                  root.selectedIndex = 0
                  root.selectCurrentOnRebuild = root.tab === "tasks"
                  root.rebuildDisplay()
                }
              }
            }
          }
        }

        Rectangle {
          id: headerField
          width: parent.width
          height: root.headerHeight
          radius: root.cornerRadius
          readonly property bool focused: (root.screen === "new" && root.formFocus === "name")
            || root.screen === "rename"
          color: headerField.focused ? root.selectedBackground : "transparent"

          Text {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            anchors.leftMargin: headerField.focused ? Style.space(12) : 0
            anchors.rightMargin: headerField.focused ? Style.space(12) : 0
            text: {
              const caret = headerField.focused && headerCaret.lit ? "▌" : ""
              if (root.screen === "new") return (root.formName || "New task name…") + caret
              if (root.screen === "rename") return (root.renameName || "Rename…") + caret
              if (root.screen === "progress") return root.progressFailed ? "Container setup failed" : "Creating container…"
              return root.filterText || (root.tab === "repos" ? "Filter repos…" : "Switch task…")
            }
            color: headerField.focused ? root.selectedText : root.foreground
            opacity: {
              if (headerField.focused) return 1
              if (root.screen === "new") return root.formName ? 1 : 0.58
              if (root.screen === "rename") return root.renameName ? 1 : 0.58
              if (root.screen === "progress") return 1
              return root.filterText ? 1 : 0.58
            }
            font.family: root.fontFamily
            font.pixelSize: Style.font.heading
            elide: Text.ElideRight
          }

          Timer {
            id: headerCaret
            interval: 530
            running: headerField.focused
            repeat: true
            property bool lit: true
            onTriggered: lit = !lit
            onRunningChanged: if (running) lit = true
          }

          MouseArea {
            anchors.fill: parent
            enabled: root.screen === "new" || root.screen === "rename"
            cursorShape: Qt.IBeamCursor
            onClicked: {
              if (root.screen === "new") root.formFocus = "name"
            }
          }
        }

        Text {
          width: parent.width
          visible: root.formError.length > 0 && root.screen !== "list"
          text: root.formError
          color: root.selectedText
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
        }

        Item {
          width: parent.width
          height: parent.height - root.headerHeight - root.footerHeight - root.contentSpacing * 2
            - (root.screen === "list" ? root.tabHeight + root.contentSpacing : 0)
            - (root.formError.length > 0 && root.screen !== "list" ? Style.font.caption + root.contentSpacing : 0)

          ListView {
            id: resultList
            anchors.fill: parent
            model: displayModel
            clip: true
            spacing: Style.space(4)
            boundsBehavior: Flickable.StopAtBounds
            visible: root.screen === "list" && displayModel.count > 0

            delegate: Rectangle {
              id: row
              required property int index
              required property string taskId
              required property string kind
              required property string status
              required property bool current
              required property string label
              required property string icon
              required property string detail

              readonly property bool hasCursor: root.cursorActive && index === root.selectedIndex

              width: ListView.view.width
              height: root.rowHeight
              radius: root.cornerRadius
              color: hasCursor ? root.selectedBackground : "transparent"

              Row {
                anchors.fill: parent
                anchors.leftMargin: Style.space(12)
                anchors.rightMargin: Style.space(12)
                spacing: Style.space(10)

                Text {
                  width: Style.space(22)
                  height: parent.height
                  text: row.current ? "●" : ""
                  color: row.hasCursor ? root.selectedText : root.foreground
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.title
                  verticalAlignment: Text.AlignVCenter
                  horizontalAlignment: Text.AlignHCenter
                }

                Text {
                  width: Style.space(28)
                  height: parent.height
                  text: row.icon
                  color: row.hasCursor ? root.selectedText : root.foreground
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.title
                  verticalAlignment: Text.AlignVCenter
                }

                Column {
                  width: parent.width - Style.space(22) - Style.space(28) - parent.spacing * 2
                  anchors.verticalCenter: parent.verticalCenter
                  spacing: 0

                  Text {
                    width: parent.width
                    text: row.label
                    color: row.hasCursor ? root.selectedText : root.foreground
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.title
                    elide: Text.ElideRight
                  }

                  Text {
                    width: parent.width
                    visible: row.detail.length > 0
                    text: row.detail
                    color: row.hasCursor ? root.selectedText : root.foreground
                    opacity: 0.62
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                    elide: Text.ElideRight
                  }
                }
              }

              MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onPositionChanged: function(mouse) { root.selectFromPointer(row.index, row, mouse) }
                onClicked: {
                  root.cursorActive = true
                  root.selectedIndex = row.index
                  root.activateIndex(row.index)
                }
              }
            }
          }

          Column {
            anchors.centerIn: parent
            spacing: Style.space(8)
            visible: root.screen === "list" && displayModel.count === 0

            Text {
              text: "󱓝"
              color: root.selectedText
              opacity: 0.8
              font.family: root.fontFamily
              font.pixelSize: Style.font.displayLarge
              horizontalAlignment: Text.AlignHCenter
              width: parent.width
            }

            Text {
              text: root.listError
                ? root.listError
                : (root.filterText ? "No matches for “" + root.filterText + "”" : (root.tab === "repos" ? "No repos — Alt+N to add" : "No tasks — Alt+N to create"))
              color: root.foreground
              opacity: 0.7
              font.family: root.fontFamily
              font.pixelSize: Style.font.title
              horizontalAlignment: Text.AlignHCenter
              width: parent.width
            }
          }

          Column {
            anchors.fill: parent
            spacing: Style.space(8)
            visible: root.screen === "new"

            Text {
              width: parent.width
              text: "Repo"
              color: root.foreground
              opacity: 0.7
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
            }

            Repeater {
              model: root.formRepoChoices

              Rectangle {
                required property int index
                required property var modelData
                width: parent.width
                height: Style.space(36)
                radius: root.cornerRadius
                readonly property bool chosen: root.formRepoIndex === index
                readonly property bool focused: chosen && root.formFocus === "repo"
                color: focused ? root.selectedBackground : "transparent"
                opacity: chosen && !focused ? 0.78 : 1

                Text {
                  anchors.fill: parent
                  anchors.leftMargin: Style.space(10)
                  anchors.rightMargin: Style.space(10)
                  text: (parent.chosen ? "●  " : "   ") + modelData.name + (modelData.path ? "  " + modelData.path : "")
                  color: parent.focused ? root.selectedText : root.foreground
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.body
                  elide: Text.ElideMiddle
                  verticalAlignment: Text.AlignVCenter
                }

                MouseArea {
                  anchors.fill: parent
                  onClicked: {
                    root.formRepoIndex = index
                    root.formFocus = "repo"
                  }
                }
              }
            }

            Toggle {
              width: parent.width
              visible: root.formRepo && root.formRepo.kind === "repo"
              label: "Worktree"
              description: root.formWorktree ? "Create a git worktree / jj workspace" : "Use the main checkout"
              checked: root.formWorktree
              hasCursor: root.formFocus === "worktree"
              foreground: root.foreground
              fontFamily: root.fontFamily
              onClicked: root.formWorktree = !root.formWorktree
            }

            Toggle {
              width: parent.width
              label: "Distrobox isolation"
              description: "Launch terminal, editor, and browser in a container"
              checked: root.formContainer
              hasCursor: root.formFocus === "container"
              foreground: root.foreground
              fontFamily: root.fontFamily
              onClicked: root.formContainer = !root.formContainer
            }
          }

          Flickable {
            anchors.fill: parent
            visible: root.screen === "progress"
            clip: true
            contentWidth: width
            contentHeight: progressText.implicitHeight
            boundsBehavior: Flickable.StopAtBounds

            Text {
              id: progressText
              width: parent.width
              text: root.progressLog
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              wrapMode: Text.WrapAnywhere
            }
          }
        }

        Text {
          width: parent.width
          height: root.footerHeight
          text: {
            if (root.screen === "new") return "Tab fields · ↑↓ repo · Space toggle · Enter create · Esc back"
            if (root.screen === "rename") return "Enter save · Esc back"
            if (root.screen === "progress") return root.progressDone ? "Enter close" : "Creating…"
            if (root.tab === "archived") return "↵ restore · ⌥n new · ⌥e rename · ⌥⇧d delete · ←→ tabs"
            if (root.tab === "repos") return "⌥n add · ⌥d remove · ←→ tabs"
            return "↵ switch · ⌥n new · ⌥e rename · ⌥d archive · ⌥⇧d delete · ←→ tabs"
          }
          color: root.foreground
          opacity: 0.55
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          elide: Text.ElideRight
          verticalAlignment: Text.AlignVCenter
        }
      }
    }
  }
}
