--
-- LAE task spaces menu for Elephant/Walker
--
Name = "laetasks"
NamePretty = "Task Spaces"
HideFromProviderlist = true
FixedOrder = true

function GetEntries()
  local entries = {}
  local home = os.getenv("HOME") or ""
  local helper = home .. "/.local/share/lae/bin/lae-task-menu-json"
  local handle = io.popen('"' .. helper .. '" 2>&1')

  if not handle then
    return entries
  end

  for line in handle:lines() do
    local label, sub, status, action = line:match("([^\t]*)\t([^\t]*)\t([^\t]*)\t(.*)")
    if label and action and action ~= "" then
      table.insert(entries, {
        Text = label .. "  ",
        Subtext = sub .. (status ~= "" and ("  [" .. status .. "]") or ""),
        Actions = {
          activate = action,
        },
      })
    end
  end

  handle:close()
  return entries
end
