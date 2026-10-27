const fmt = (b) => (!b ? "" : b >= 1073741824 ? (b / 1073741824).toFixed(2) + " GB" : b >= 1048576 ? (b / 1048576).toFixed(1) + " MB" : Math.round(b / 1024) + " KB");
const nameOf = (u) => { try { return decodeURIComponent(new URL(u).pathname.split("/").filter(Boolean).pop() || new URL(u).hostname); } catch { return u.slice(0, 60); } };

(async () => {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const res = (await chrome.runtime.sendMessage({ type: "list", tabId: tab.id }).catch(() => null)) || { items: [], running: false };
  document.getElementById("st").className = "dot " + (res.running ? "on" : "off");
  document.getElementById("stt").textContent = res.running ? "connected" : "app not running";
  const list = document.getElementById("list");
  const note = (t) => (list.innerHTML = `<div class="empty"></div>`) && (list.firstChild.textContent = t);

  // YouTube, Facebook, Instagram, X …: the formats come from yt-dlp (run by the desktop app); these
  // pages have nothing to sniff. `force` asks yt-dlp about any page.
  async function showYtdlp(url, force) {
    note("Getting the format list from yt-dlp…");
    const yt = (await chrome.runtime.sendMessage({ type: "ytdlp", url, force: !!force }).catch(() => null)) || { ok: false };
    if (!yt.ok) return note(yt.error || "Couldn’t reach DownCraft.");
    list.innerHTML = "";
    for (const ent of dcEntries(yt)) {
      const head = document.createElement("div");
      head.className = "empty";
      head.textContent = ent.title || ent.uploader || "Video";
      list.append(head);
      for (const c of dcChoices(ent)) {
        const row = document.createElement("div");
        row.className = "it";
        const n = document.createElement("span");
        n.className = "n";
        n.textContent = c.text;
        n.title = c.tip;
        const s = document.createElement("small");
        s.textContent = c.size ? (c.approx ? "~" : "") + fmt(c.size) : "";
        const b = document.createElement("button");
        b.textContent = "Download";
        b.onclick = async () => {
          const r = await chrome.runtime.sendMessage({ type: "send-format", format: c.f, audio: c.a, title: ent.title, uploader: ent.uploader, tabId: tab.id, pageUrl: url });
          b.textContent = r && r.ok ? "Sent ✓" : "Failed";
        };
        row.append(n, s, b);
        list.append(row);
      }
    }
  }

  const page = dcVideoPage(tab.url || "");
  if (page) return showYtdlp(page);
  if (!res.items.length) {
    list.innerHTML = '<div class="empty">No media detected on this page yet.</div>';
    // nothing sniffed: yt-dlp knows well over a thousand sites, so offer it
    if (/^https?:\/\//i.test(tab.url || "")) {
      const b = document.createElement("button");
      b.textContent = "Try with yt-dlp";
      b.style.margin = "8px 12px";
      b.onclick = () => showYtdlp(tab.url.split("#")[0], true);
      list.append(b);
    }
    return;
  }
  for (const it of res.items) {
    const row = document.createElement("div");
    row.className = "it";
    const n = document.createElement("span");
    n.className = "n";
    n.title = it.url;
    n.textContent = nameOf(it.url);
    const s = document.createElement("small");
    const dash = /\.mpd(\?|$)/i.test(it.url);
    s.textContent = it.stream ? (dash ? "DASH" : "HLS stream") : fmt(it.size);
    const b = document.createElement("button");
    b.textContent = dash ? "n/a" : "Download";
    b.disabled = dash;
    b.onclick = async () => {
      const r = await chrome.runtime.sendMessage({ type: "send", url: it.url, tabId: tab.id, referer: tab.url });
      b.textContent = r.ok ? "Sent ✓" : "Failed";
    };
    row.append(n, s, b);
    list.append(row);
  }
})();
