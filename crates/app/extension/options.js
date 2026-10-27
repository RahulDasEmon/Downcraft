const DEFAULTS = { intercept: true, minSizeMB: 0, onlyExt: "", skipHosts: "" };
const $ = (id) => document.getElementById(id);
(async () => {
  const c = { ...DEFAULTS, ...(await chrome.storage.sync.get(DEFAULTS)) };
  $("intercept").checked = c.intercept;
  $("onlyExt").value = c.onlyExt;
  $("minSizeMB").value = c.minSizeMB;
  $("skipHosts").value = c.skipHosts;
})();
$("save").onclick = async () => {
  await chrome.storage.sync.set({
    intercept: $("intercept").checked,
    onlyExt: $("onlyExt").value,
    minSizeMB: Math.max(0, Number($("minSizeMB").value) || 0),
    skipHosts: $("skipHosts").value,
  });
  $("msg").textContent = "Saved ✓";
};
