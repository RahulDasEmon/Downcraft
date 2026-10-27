importScripts("formats.js");
// DownCraft – service worker: sniffs media requests per tab and talks to the desktop app.
const API = "http://127.0.0.1:19870";
const MEDIA_EXT = /\.(mp4|m4v|webm|mkv|mov|flv|avi|mp3|m4a|aac|ogg|opus|wav|flac)(\?|#|$)/i;
const STREAM_EXT = /\.(m3u8|mpd)(\?|#|$)/i;
const MIN_SIZE = 150 * 1024; // ignore tiny clips / segments
const MAX_PER_TAB = 25;

const key = (tabId) => `t${tabId}`;

async function getItems(tabId) {
  const r = await chrome.storage.session.get(key(tabId));
  return r[key(tabId)] || [];
}
// Several responses for one tab can arrive at once; every read-modify-write goes through this queue
// so none of them overwrites another's result.
let chain = Promise.resolve();
const serial = (fn) => (chain = chain.then(fn, fn));

async function setItems(tabId, items) {
  await chrome.storage.session.set({ [key(tabId)]: items });
  const n = visible(items).length;
  try {
    await chrome.action.setBadgeBackgroundColor({ tabId, color: "#1e7a1e" });
    await chrome.action.setBadgeText({ tabId, text: n ? String(n) : "" });
  } catch {}
}

// A master playlist lists the quality variants; DownCraft picks the best one itself.
async function isMasterPlaylist(url) {
  try {
    const r = await fetch(url, { credentials: "include" });
    const t = (await r.text()).slice(0, 200000);
    return t.includes("#EXT-X-STREAM-INF");
  } catch {
    return false;
  }
}

// Hide variant/media playlists when the page's master playlist was seen.
function visible(items) {
  const hasMaster = items.some((i) => i.stream && i.master);
  return items.filter((i) => !i.stream || !hasMaster || i.master);
}

function header(details, name) {
  const h = (details.responseHeaders || []).find((x) => x.name.toLowerCase() === name);
  return h ? h.value || "" : "";
}

chrome.webRequest.onHeadersReceived.addListener(
  async (d) => {
    if (d.tabId < 0 || d.statusCode >= 400) return;
    const ct = header(d, "content-type").toLowerCase().split(";")[0].trim();
    const cr = header(d, "content-range");
    let size = parseInt(header(d, "content-length") || "0", 10) || 0;
    const m = cr.match(/\/(\d+)$/);
    if (m) size = parseInt(m[1], 10);

    const stream = STREAM_EXT.test(d.url) || ct.includes("mpegurl") || ct === "application/dash+xml";
    const media =
      !stream &&
      ct !== "video/mp2t" && // HLS segments
      (ct.startsWith("video/") || ct.startsWith("audio/") || d.type === "media" || (MEDIA_EXT.test(d.url) && !ct.startsWith("text/")));
    if (!stream && !media) return;
    if (media && size && size < MIN_SIZE) return;

    let master = false;
    if (stream) master = await isMasterPlaylist(d.url);
    await serial(async () => {
      const items = await getItems(d.tabId);
      if (items.some((i) => i.url === d.url)) return;
      items.unshift({ url: d.url, ct, size, stream, master, ts: Date.now() });
      await setItems(d.tabId, items.slice(0, MAX_PER_TAB));
    });
  },
  { urls: ["<all_urls>"] },
  ["responseHeaders"]
);

chrome.tabs.onUpdated.addListener((tabId, info) => {
  if (info.status === "loading" && info.url) serial(() => setItems(tabId, []));
});
chrome.tabs.onRemoved.addListener((tabId) => chrome.storage.session.remove(key(tabId)));

async function ping() {
  try {
    const r = await fetch(API + "/ping");
    return r.ok;
  } catch {
    return false;
  }
}

function fileHint(title, url) {
  const t = (title || "").replace(/[<>:"/\\|?*\x00-\x1f]/g, " ").replace(/\s+/g, " ").trim().slice(0, 120);
  if (!t) return null;
  let ext = "mp4";
  try {
    const m = new URL(url).pathname.match(/\.([a-z0-9]{2,5})$/i);
    if (m) ext = m[1].toLowerCase();
  } catch {}
  if (ext === "m3u8" || ext === "m3u" || ext === "mpd") ext = "ts";
  return `${t}.${ext}`;
}

async function send({ url, tabId, referer, size = 0, source = "media", fileName, headers: given, cookies = true, audio }) {
  let tab = null;
  try {
    tab = await chrome.tabs.get(tabId);
  } catch {}
  const page = referer || (tab && tab.url) || "";
  const headers = { "User-Agent": navigator.userAgent };
  if (page) {
    headers.Referer = page;
    try {
      headers.Origin = new URL(page).origin;
    } catch {}
  }
  if (cookies) {
    try {
      const cs = await chrome.cookies.getAll({ url });
      if (cs.length) headers.Cookie = cs.map((c) => `${c.name}=${c.value}`).join("; ");
    } catch {}
  }
  // the page the download was started from, shown in DownCraft's "Parent web page" column (never sent to servers)
  const parent = (tab && tab.url) || page;
  if (parent) headers["X-DownCraft-Page"] = parent;
  if (given) Object.assign(headers, given);
  try {
    const r = await fetch(API + "/add", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ url, file_name: fileName || fileHint(tab && tab.title, url), headers, size, source, audio }),
    });
    return r.ok;
  } catch {
    return false;
  }
}

// ───────────── yt-dlp: format lists for YouTube, Facebook, Instagram, X … ─────────────
// These players use blob: / MSE (or split streams), so there is no media link to sniff. Instead the
// desktop app runs yt-dlp for the page and returns the formats; the one picked here is sent to /add
// like any other link.
const YT_TTL = 5 * 60 * 1000; // links are valid for hours; this only spares repeated yt-dlp runs
const ytCache = new Map(); // page url -> { ts, promise }

// The browser's cookies for a page, so posts that need a sign-in (Instagram, Facebook, private X
// accounts) work. YouTube is left out on purpose: its links carry their own authorisation.
async function pageCookies(url) {
  if (dcYtPage(url)) return [];
  try {
    const cs = await chrome.cookies.getAll({ url });
    return cs.map((c) => ({ domain: c.domain, path: c.path, secure: c.secure, expires: c.expirationDate || 0, name: c.name, value: c.value }));
  } catch {
    return [];
  }
}

async function fetchFormats(url) {
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), 100000);
  try {
    const r = await fetch(API + "/ytdlp", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ url, cookies: await pageCookies(url) }),
      signal: ctl.signal,
    });
    return await r.json();
  } catch (e) {
    return e && e.name === "AbortError"
      ? { ok: false, error: "yt-dlp took too long to answer." }
      : { ok: false, offline: true, error: "DownCraft isn’t running. Start it and try again." };
  } finally {
    clearTimeout(timer);
  }
}

// `force`: ask yt-dlp about any page (the fallback when nothing could be sniffed); otherwise only
// the sites it is meant for.
function ytFormats(pageUrl, force) {
  let url = dcVideoPage(pageUrl);
  if (!url && force && /^https?:\/\//i.test(pageUrl || "")) url = pageUrl.split("#")[0];
  if (!url) return Promise.resolve({ ok: false, error: "yt-dlp isn’t used for this page." });
  const hit = ytCache.get(url);
  if (hit && Date.now() - hit.ts < YT_TTL) return hit.promise;
  const promise = fetchFormats(url);
  ytCache.set(url, { ts: Date.now(), promise });
  promise.then((r) => r && r.ok === false && ytCache.delete(url)); // never cache a failure
  return promise;
}

const cleanName = (t) => (t || "").replace(/[<>:"/\\|?*\x00-\x1f]/g, " ").replace(/\s+/g, " ").trim().slice(0, 120);

function ytLabel(f) {
  if (f.kind === "audio") return f.ext === "m4a" ? "audio" : `audio ${f.ext}`;
  return `${f.height ? f.height + "p" : f.note || "video"}${f.fps > 30 ? f.fps : ""}`;
}

async function sendFormat({ format: f, audio: a, title, uploader, tabId, pageUrl }) {
  const merged = !!a && f.kind === "video";
  const ext = merged ? dcOutExt(f, a) : f.hls ? "ts" : f.ext || "mp4";
  const base = cleanName(title) || cleanName(uploader) || "video";
  // yt-dlp's own request headers (User-Agent …) go with the signed link; the site's Referer/Origin are
  // added, and the browser's cookies are deliberately not forwarded – the link carries its own authorisation.
  let origin = "https://www.youtube.com";
  try {
    origin = new URL(pageUrl).origin;
  } catch {}
  const hdrs = (x) => ({ ...(x.headers || {}), Referer: origin + "/", Origin: origin });
  return send({
    url: f.url,
    tabId,
    referer: origin + "/",
    size: f.size || 0,
    source: "ytdlp",
    fileName: `${base} [${ytLabel(f)}].${ext}`,
    headers: hdrs(f),
    audio: merged ? { url: a.url, headers: hdrs(a), size: a.size || 0, ext: a.ext || "m4a" } : undefined,
    cookies: false,
  });
}

chrome.runtime.onMessage.addListener((msg, sender, respond) => {
  const tabId = msg.tabId ?? (sender.tab && sender.tab.id);
  (async () => {
    if (msg.type === "list") respond({ items: visible(await getItems(tabId)), running: await ping() });
    else if (msg.type === "send") respond({ ok: await send({ url: msg.url, tabId, referer: msg.referer }) });
    else if (msg.type === "ytdlp") respond(await ytFormats(msg.url, !!msg.force));
    else if (msg.type === "send-format") respond({ ok: await sendFormat({ ...msg, tabId }) });
    else if (msg.type === "ping") respond({ running: await ping() });
    else if (msg.type === "bypass") (bypassUntil = Date.now() + 4000), respond({});
    else respond({});
  })();
  return true; // async response
});


// ───────────── taking over browser downloads (like FluxDown's interception layer) ─────────────
const DEFAULTS = { intercept: true, minSizeMB: 0, onlyExt: "", skipHosts: "" };
const getSettings = async () => ({ ...DEFAULTS, ...(await chrome.storage.sync.get(DEFAULTS)) });
let bypassUntil = 0; // set by Alt+click: the next downloads are left to the browser

const list = (v) => v.split(/[\s,;]+/).map((x) => x.trim().toLowerCase().replace(/^\./, "")).filter(Boolean);

function wanted(cfg, item) {
  if (!cfg.intercept || Date.now() < bypassUntil) return false;
  let u;
  try {
    u = new URL(item.finalUrl || item.url);
  } catch {
    return false;
  }
  if (u.protocol !== "http:" && u.protocol !== "https:") return false; // blob:, data:, file: stay with the browser
  const host = u.hostname.toLowerCase();
  if (list(cfg.skipHosts).some((h) => host === h || host.endsWith("." + h))) return false;
  const exts = list(cfg.onlyExt);
  if (exts.length) {
    const name = (item.filename || u.pathname).toLowerCase();
    if (!exts.some((e) => name.endsWith("." + e))) return false;
  }
  const min = cfg.minSizeMB * 1048576;
  if (min > 0 && item.fileSize > 0 && item.fileSize < min) return false;
  return true;
}

chrome.downloads.onCreated.addListener(async (item) => {
  try {
    const cfg = await getSettings();
    if (!wanted(cfg, item)) return;
    if (!(await ping())) return; // app not running: let the browser keep the download
    const url = item.finalUrl || item.url;
    const base = (item.filename || "").split(/[\\/]/).pop();
    // cancel first so the browser never starts writing the file
    await chrome.downloads.cancel(item.id);
    await chrome.downloads.erase({ id: item.id });
    const ok = await send({
      url,
      tabId: item.tabId ?? -1,
      referer: item.referrer || undefined,
      size: item.fileSize > 0 ? item.fileSize : 0,
      source: "download",
      fileName: base || undefined,
    });
    if (!ok) chrome.downloads.download({ url }); // hand it back rather than lose it
  } catch {}
});

// ───────────── right-click → Download with DownCraft ─────────────
chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "fc-send",
    title: "Download with DownCraft",
    contexts: ["link", "video", "audio"],
  });
});
chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const url = info.linkUrl || info.srcUrl;
  if (!url || !/^https?:/i.test(url)) return;
  const ok = await send({ url, tabId: tab ? tab.id : -1, referer: info.pageUrl, source: "menu" });
  try {
    await chrome.action.setBadgeText({ tabId: tab.id, text: ok ? "✓" : "!" });
    setTimeout(() => chrome.action.setBadgeText({ tabId: tab.id, text: "" }), 2500);
  } catch {}
});
