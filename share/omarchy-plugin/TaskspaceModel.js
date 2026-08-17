function parseTaskList(text) {
  var parsed = JSON.parse(text || "[]")
  if (Array.isArray(parsed))
    return { active: parsed, archived: [] }
  return {
    active: Array.isArray(parsed.active) ? parsed.active : [],
    archived: Array.isArray(parsed.archived) ? parsed.archived : []
  }
}

function parseRepos(text) {
  try {
    var parsed = JSON.parse(text || "[]")
    if (Array.isArray(parsed)) return parsed
  } catch (e) {
  }
  return parseRepoText(text)
}

function parseRepoText(text) {
  var lines = String(text || "").split("\n")
  var out = []
  for (var i = 0; i < lines.length; i++) {
    var line = lines[i]
    if (!line || line.indexOf("No repos") === 0) continue
    var id = line.substring(0, 20).trim()
    var rest = line.substring(22)
    var split = rest.indexOf("  ")
    if (split < 0) continue
    out.push({
      id: id,
      name: rest.substring(0, split),
      path: rest.substring(split + 2),
      url: null
    })
  }
  return out
}

function rowLabel(item) {
  if (!item) return ""
  if (item.kind === "new-task") return "New Task"
  if (item.kind === "new-repo") return "New Repo"
  if (item.kind === "default") return "default taskspace"
  if (item.kind === "repo") return item.name || item.path || item.id || ""
  return item.name || item.id || ""
}

function rowIcon(item) {
  if (!item) return ""
  if (item.kind === "new-task" || item.kind === "new-repo") return "󰐕"
  if (item.kind === "default") return "󰣇"
  if (item.kind === "repo") return "󰣞"
  if (item.status === "archived") return "󰀼"
  return "󱓝"
}

function rowDetail(item) {
  if (!item) return ""
  if (item.kind === "new-task" || item.kind === "new-repo" || item.kind === "default") return ""
  if (item.kind === "repo") return item.path || ""
  return item.repo_name || ""
}

function matches(item, query) {
  if (!query) return true
  var hay = (rowLabel(item) + " " + (item.id || "") + " " + (item.path || "") + " " + (item.repo_name || "")).toLowerCase()
  return hay.indexOf(query) !== -1
}

function listedAtMs(item) {
  var raw = item && item.listed_at
  if (!raw) return 0
  var ms = Date.parse(raw)
  return isNaN(ms) ? 0 : ms
}

function sortItems(items) {
  return items.slice().sort(function(left, right) {
    if (left.kind === "default" && right.kind !== "default") return -1
    if (right.kind === "default" && left.kind !== "default") return 1
    var listed = listedAtMs(right) - listedAtMs(left)
    if (listed !== 0) return listed
    var nameCmp = String(rowLabel(left)).toLowerCase().localeCompare(String(rowLabel(right)).toLowerCase())
    if (nameCmp !== 0) return nameCmp
    return String(left.id || "").localeCompare(String(right.id || ""))
  })
}

function createdTaskId(text) {
  var match = String(text || "").match(/Created task\s+(\S+)/)
  return match ? match[1] : ""
}
