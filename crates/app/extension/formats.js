// Shared by the page button (content.js), the toolbar popup (popup.js) and the background worker:
// turns yt-dlp's format list into the rows the user picks from.
//
// YouTube only serves its better pictures without sound. Such a row carries the best matching audio
// (`a`); the desktop app downloads both and joins them with ffmpeg.

// container of the merged file
function dcOutExt(v, a) {
  if (v.ext === "mp4" && a.ext === "m4a") return "mp4";
  if (v.ext === "webm" && a.ext === "webm") return "webm";
  return "mkv";
}

function dcPickAudio(v, audios) {
  const want = v.ext === "mp4" ? "m4a" : v.ext === "webm" ? "webm" : "";
  const score = (a) =>
    (a.ext === want ? 2e6 : 0) + (/default|original/i.test(a.note || "") ? 1e6 : 0) + (a.lang ? 0 : 1e5) + (a.tbr || 0);
  return audios.slice().sort((x, y) => score(y) - score(x))[0] || null;
}

const DC_CODEC_RANK = { "H.264": 3, VP9: 2, AV1: 1 }; // what plays everywhere comes first

function dcQualityTag(h) {
  return h >= 2160 ? " 4K" : h >= 720 ? " HD" : "";
}

// -> [{ kind: "video" | "audio", f, a, ext, text, size, approx, tip }]  highest quality first
function dcChoices(res) {
  const all = (res && res.formats) || [];
  const audios = all.filter((f) => f.kind === "audio");
  const out = [];

  // one row per quality (height + 60fps flag)
  const groups = new Map();
  for (const f of all) {
    if (f.kind === "audio") continue;
    const key = (f.height || 0) + "p" + (f.fps > 30 ? 60 : 30);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(f);
  }
  for (const list of groups.values()) {
    const ok = list.filter((f) => !f.hls);
    let v = ok.filter((f) => f.kind === "muxed").sort((x, y) => (y.tbr || 0) - (x.tbr || 0))[0];
    let a = null;
    if (!v) {
      v = ok
        .filter((f) => f.kind === "video")
        .sort((x, y) => (DC_CODEC_RANK[y.codec] || 0) - (DC_CODEC_RANK[x.codec] || 0) || (y.tbr || 0) - (x.tbr || 0))[0];
      if (v) a = dcPickAudio(v, audios);
      if (v && !a) v = null; // never offer a silent file
    }
    if (!v) v = list.filter((f) => f.kind === "muxed").sort((x, y) => (y.tbr || 0) - (x.tbr || 0))[0]; // HLS with sound
    if (!v) continue;
    const ext = a ? dcOutExt(v, a) : v.hls ? "ts" : v.ext || "mp4";
    const q = v.height ? v.height + "p" + (v.fps > 30 ? v.fps : "") + dcQualityTag(v.height) : v.note || "video";
    out.push({
      kind: "video",
      f: v,
      a,
      ext,
      height: v.height || 0,
      fps: v.fps || 0,
      text: ext.toUpperCase() + " file, quality " + q,
      size: (v.size || 0) + (a ? a.size || 0 : 0),
      approx: !!(v.approx || (a && a.approx) || a),
      tip: [v.codec, v.hdr ? "HDR" : "", a ? "+ " + a.ext.toUpperCase() + " audio (merged)" : ""].filter(Boolean).join(" · "),
    });
  }
  out.sort((x, y) => y.height - x.height || y.fps - x.fps); // best quality first

  // audio only: the best track of each type (and language), highest bitrate first
  const seen = new Map();
  for (const f of audios) {
    const key = f.ext + "|" + (f.lang || "");
    const cur = seen.get(key);
    const rank = (x) => (/default|original/i.test(x.note || "") ? 1e6 : 0) + (x.tbr || 0);
    if (!cur || rank(f) > rank(cur)) seen.set(key, f);
  }
  const rows = [...seen.values()]
    .sort((x, y) => (y.tbr || 0) - (x.tbr || 0)) // highest bitrate first
    .slice(0, 4)
    .map((f) => ({
      kind: "audio",
      f,
      a: null,
      ext: f.ext,
      text: f.ext.toUpperCase() + " audio" + (f.lang ? " (" + f.lang + ")" : "") + (f.tbr ? ", " + Math.round(f.tbr) + "k" : ""),
      size: f.size || 0,
      approx: !!f.approx,
      tip: f.codec || "",
    }));
  return out.concat(rows);
}

// ───────────── which pages go to yt-dlp ─────────────
// YouTube plays through blob: video and the big social sites (Facebook, Instagram, X …) hand out
// short-lived or split streams, so for these the desktop app asks yt-dlp instead of sniffing links.
const DC_YT_HOST = /(^|\.)(youtube\.com|youtube-nocookie\.com)$/i;
const DC_SITES = new RegExp(
  "(^|\\.)(" +
    [
      "facebook\\.com", "fb\\.watch", "fb\\.com", "instagram\\.com", "twitter\\.com", "x\\.com", "threads\\.net", "tiktok\\.com",
      "vimeo\\.com", "twitch\\.tv", "dailymotion\\.com", "reddit\\.com", "bilibili\\.com", "rumble\\.com", "streamable\\.com",
      "snapchat\\.com", "pinterest\\.com", "linkedin\\.com", "soundcloud\\.com", "odysee\\.com", "ok\\.ru", "vk\\.com", "kick\\.com", "bsky\\.app",
    ].join("|") +
    ")$",
  "i"
);

// Canonical watch URL of the video on a YouTube page (playlist/mix parameters dropped), or null.
function dcYtPage(url) {
  try {
    const u = new URL(url);
    let id = null;
    if (u.hostname.toLowerCase() === "youtu.be") id = u.pathname.split("/")[1];
    else if (DC_YT_HOST.test(u.hostname)) {
      const m = u.pathname.match(/^\/(?:shorts|live|embed|v)\/([\w-]{6,})/);
      if (u.pathname === "/watch") id = u.searchParams.get("v");
      else if (m) id = m[1];
    }
    return id && /^[\w-]{6,}$/.test(id) ? `https://www.youtube.com/watch?v=${id}` : null;
  } catch {
    return null;
  }
}

// true for the sites above (not YouTube, which has its own canonical form)
function dcIsSocialSite(url) {
  try {
    const u = new URL(url);
    return /^https?:$/.test(u.protocol) && DC_SITES.test(u.hostname) && !DC_YT_HOST.test(u.hostname);
  } catch {
    return false;
  }
}

// Address to give yt-dlp for this page, or null when the page isn't one yt-dlp should handle by default.
function dcVideoPage(url) {
  const yt = dcYtPage(url);
  if (yt) return yt;
  if (!dcIsSocialSite(url)) return null;
  const u = new URL(url);
  u.hash = "";
  return u.href;
}

// A result holds one video, or several (`entries`) for a post such as an X thread or an Instagram carousel.
function dcEntries(res) {
  return res && Array.isArray(res.entries) && res.entries.length ? res.entries : res && res.ok ? [res] : [];
}
