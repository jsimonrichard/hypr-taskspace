-- Hypr Taskspace Omarchy (Quattro) bindings.
-- Installed via a marked `dofile` in ~/.config/hypr/bindings.lua.
-- Unbind Omarchy defaults before rebinding (see omarchy Hyprland skill).

local tsk = "@TSK_CMD@"

local function run(args)
  return tsk .. " " .. args
end

for workspace = 1, 10 do
  local key = "code:" .. tostring(workspace + 9)
  hl.unbind("SUPER + " .. key)
  hl.unbind("SUPER + SHIFT + " .. key)
end

for digit = 1, 9 do
  hl.unbind("SUPER + " .. digit)
  hl.unbind("SUPER + SHIFT + " .. digit)
end
hl.unbind("SUPER + 0")
hl.unbind("SUPER + SHIFT + 0")

hl.unbind("SUPER + TAB")
hl.unbind("SUPER + SHIFT + TAB")
hl.unbind("SUPER + CTRL + TAB")
hl.unbind("SUPER + mouse_down")
hl.unbind("SUPER + mouse_up")
hl.unbind("SUPER + RETURN")
hl.unbind("SUPER + Return")
hl.unbind("SUPER + E")
hl.unbind("SUPER + e")
hl.unbind("SUPER + B")
hl.unbind("SUPER + b")

hl.unbind("SUPER + SHIFT + B")
hl.unbind("SUPER + SHIFT + RETURN")
hl.unbind("SUPER + SHIFT + ALT + B")

-- SUPER+H is unbound on stock Omarchy; bind without an unbind.
o.bind("SUPER + H", "Default taskspace", run("taskspace default"))

for workspace = 1, 10 do
  local key = "code:" .. tostring(workspace + 9)
  local n = tostring(workspace)
  o.bind("SUPER + " .. key, "Taskspace workspace " .. n, run("workspace switch " .. n))
  o.bind("SUPER + SHIFT + " .. key, "Move to taskspace workspace " .. n, run("workspace move-dispatch " .. n))
end

for digit = 1, 9 do
  local n = tostring(digit)
  o.bind("SUPER + " .. n, "Taskspace workspace " .. n, run("workspace switch " .. n))
  o.bind("SUPER + SHIFT + " .. n, "Move to taskspace workspace " .. n, run("workspace move-dispatch " .. n))
end
o.bind("SUPER + 0", "Taskspace workspace 10", run("workspace switch 10"))
o.bind("SUPER + SHIFT + 0", "Move to taskspace workspace 10", run("workspace move-dispatch 10"))

o.bind("SUPER + bracketleft", "Previous workspace", run("workspace prev"))
o.bind("SUPER + bracketright", "Next workspace", run("workspace next"))
o.bind("SUPER + mouse_up", "Next workspace", run("workspace next"))
o.bind("SUPER + mouse_down", "Previous workspace", run("workspace prev"))

hl.gesture({
  fingers = 3,
  direction = "left",
  action = function()
    os.execute(run("workspace next --no-wrap") .. " >/dev/null 2>&1 &")
  end,
})
hl.gesture({
  fingers = 3,
  direction = "right",
  action = function()
    os.execute(run("workspace prev --no-wrap") .. " >/dev/null 2>&1 &")
  end,
})

o.bind("SUPER + TAB", "Task manager", run("task tui-launch"))
o.bind("SUPER + RETURN", "Task terminal", run("task terminal"))
o.bind("SUPER + E", "Task editor", run("task editor"))
o.bind("SUPER + B", "Task browser", run("task browser"))

o.bind("SUPER + SHIFT + B", "Taskspace launch browser", run("launch chromium.desktop"))
o.bind("SUPER + SHIFT + RETURN", "Taskspace launch browser", run("launch chromium.desktop"))
o.bind("SUPER + SHIFT + ALT + B", "Taskspace launch browser (private)", run("launch chromium.desktop --incognito"))

-- Window rules: Hyprland 0.55+ Lua (`float` / `center` / `size` are static effects).
-- https://wiki.hypr.land/Configuring/Basics/Window-Rules/
o.window("org.tsk.task-tui", { float = true, center = true, size = { 880, 520 } })
o.window({ title = "^tsk tasks$" }, { float = true, center = true, size = { 880, 520 } })

-- Overlay task switcher (tsk.taskspace). Same no-anim treatment as omarchy.menu.
hl.layer_rule({ match = { namespace = "tsk-taskspace" }, no_anim = true, animation = "none" })
