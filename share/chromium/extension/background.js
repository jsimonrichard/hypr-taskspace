const HOST = "org.tsk.browser";
const DEBOUNCE_MS = 400;
let debounceTimer = null;

function collectWindows(windows) {
  return windows
    .filter((w) => w.type === "normal")
    .map((w) => ({
      id: w.id,
      focused: Boolean(w.focused),
      tabs: (w.tabs || [])
        .filter((t) => t.url && !t.url.startsWith("chrome-extension://"))
        .map((t) => ({
          url: t.url,
          title: t.title || "",
          active: Boolean(t.active),
        })),
    }))
    .filter((w) => w.tabs.length > 0);
}

function pushSnapshot() {
  chrome.windows.getAll({ populate: true }, (windows) => {
    if (chrome.runtime.lastError) {
      return;
    }
    chrome.runtime.sendNativeMessage(
      HOST,
      { op: "windows", windows: collectWindows(windows) },
      () => {
        void chrome.runtime.lastError;
      }
    );
  });
}

function scheduleSnapshot() {
  if (debounceTimer) {
    clearTimeout(debounceTimer);
  }
  debounceTimer = setTimeout(pushSnapshot, DEBOUNCE_MS);
}

chrome.runtime.onInstalled.addListener(pushSnapshot);
chrome.runtime.onStartup.addListener(pushSnapshot);
chrome.tabs.onUpdated.addListener((_id, change) => {
  if (change.url || change.title || change.status === "complete") {
    scheduleSnapshot();
  }
});
chrome.tabs.onRemoved.addListener(scheduleSnapshot);
chrome.windows.onCreated.addListener(scheduleSnapshot);
chrome.windows.onRemoved.addListener(scheduleSnapshot);
chrome.windows.onFocusChanged.addListener(scheduleSnapshot);

chrome.alarms.create("tsk-snapshot", { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === "tsk-snapshot") {
    pushSnapshot();
  }
});
