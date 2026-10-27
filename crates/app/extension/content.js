// DownCraft – puts a "Download this video" button in the top-right corner of the video under the mouse.
(() => {
  if (window.__downcraftInjected) return;
  window.__downcraftInjected = true;

  const host = document.createElement("div");
  host.style.cssText = "all:initial;position:fixed;top:0;left:0;width:0;height:0;z-index:2147483647;";
  const root = host.attachShadow({ mode: "closed" });
  root.innerHTML = `
  <style>
    .wrap{position:fixed;display:none;width:230px;font:12px/1 "Segoe UI",Tahoma,Arial,sans-serif;color:#10243f;user-select:none}
    .bar{position:relative;width:230px;height:30px;filter:drop-shadow(0 1px 3px #0007);cursor:grab;touch-action:none}
    .bar.drag{cursor:grabbing}
    .bar img{display:block;width:100%;height:100%;pointer-events:none;-webkit-user-drag:none}
    button{all:unset;cursor:pointer;box-sizing:border-box;position:absolute;top:2px;bottom:2px;border-radius:4px}
    button:hover{background:#ffffff40}
    button:active{background:#00000022}
    .dl{left:1%;width:81%}
    .q{left:83.9%;width:6.8%}
    .x{left:91.6%;width:6.8%}
.menu{margin-top:2px;width:max-content;min-width:230px;max-width:min(640px,calc(100vw - 16px));max-height:320px;overflow:auto;
          background:#2b2b2b;color:#e8e8e8;font-size:12.5px;line-height:1.2;border:1px solid #3d3d3d;border-radius:5px;
          box-shadow:0 6px 18px #000b;padding:4px 0;display:none;scrollbar-width:thin;scrollbar-color:#555 #2b2b2b}
    .menu div{padding:6px 14px;cursor:pointer;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
    .menu div:hover{background:#3c3c3c}
    .menu .row{display:flex;gap:6px;align-items:baseline}
    .menu .row .no{flex:none;opacity:.8;min-width:1.6em}
    .menu .row .tt{flex:1 1 auto;min-width:0;overflow:hidden;text-overflow:ellipsis}
    .menu .row .inf{flex:none;margin-left:6px}
    .menu .row small{flex:none;opacity:.55;margin-left:10px;font-size:11px}
    .menu .sep{height:1px;padding:0;margin:4px 10px;background:#3d3d3d;cursor:default}
    .menu .sep:hover{background:#3d3d3d}
    .menu .msg{padding:8px 14px;cursor:default;opacity:.85}
    .menu .msg:hover{background:none}
    .menu .ttl{padding:6px 14px;cursor:default;opacity:.8}
    .menu .ttl:hover{background:none}
    .toast{margin-top:4px;padding:6px 9px;background:#1f2430;color:#fff;border-radius:4px;display:none;max-width:300px;line-height:1.35;
           box-shadow:0 2px 8px #0008;white-space:normal}
  </style>
  <div class="wrap">
    <div class="bar">
      <img alt="" draggable="false">
      <button class="dl" title="Download this video"></button>
      <button class="q" title="Help"></button>
      <button class="x" title="Hide on this page"></button>
    </div>
    <div class="menu"></div>
    <div class="toast"></div>
  </div>`;
  const wrap = root.querySelector(".wrap");
  const dl = root.querySelector(".dl");
  const xBtn = root.querySelector(".x");
  const qBtn = root.querySelector(".q");
  root.querySelector(".bar img").src = chrome.runtime.getURL("button.png");
  const menu = root.querySelector(".menu");
  const toastEl = root.querySelector(".toast");

  let target = null;
  let hideTimer = null;
  let dismissed = false;
  let hovering = false;
  let pointer = { x: -1, y: -1 };

  const attach = () => (document.documentElement || document.body).appendChild(host);
  attach();

  const fmtSize = (b) => (!b ? "" : b >= 1073741824 ? (b / 1073741824).toFixed(2) + " GB" : b >= 1048576 ? (b / 1048576).toFixed(1) + " MB" : Math.round(b / 1024) + " KB");
  const nameOf = (u) => {
    try {
      const p = new URL(u).pathname.split("/").filter(Boolean).pop() || new URL(u).hostname;
      return decodeURIComponent(p);
    } catch {
      return u.slice(0, 50);
    }
  };

  function rectOf(v) {
    const r = v.getBoundingClientRect();
    return r.width >= 200 && r.height >= 120 && r.bottom > 0 && r.top < innerHeight && r.right > 0 && r.left < innerWidth ? r : null;
  }
  function pickVideo(x, y) {
    let best = null,
      score = 0;
    for (const v of document.querySelectorAll("video")) {
      const r = rectOf(v);
      if (!r) continue;
      const inside = x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
      if (!inside) continue;
      const s = r.width * r.height;
      if (s > score) (score = s), (best = v);
    }
    return best;
  }

  // the box the button sticks to: the whole player when there is one (a video inside it can be
  // letter-boxed, which would push the button down), otherwise the video itself
  function anchorRect(v) {
    const p = v.closest(".html5-video-player") || v.closest("#movie_player");
    if (p) {
      const pr = p.getBoundingClientRect();
      if (pr.width >= 200 && pr.height >= 120) return { left: pr.left, right: pr.right, top: pr.top - 16, player: true };
    }
    return rectOf(v);
  }
  // position chosen by dragging (kept for the page's lifetime and remembered between visits)
  let manual = null;
  try {
    chrome.storage.local.get("dcBtnPos").then((r) => {
      const p = r && r.dcBtnPos;
      if (p && typeof p.fx === "number" && typeof p.fy === "number") manual = p;
    }).catch(() => {});
  } catch {}
  function place() {
    if (!target || !target.isConnected) return hide();
    if (!rectOf(target)) return hide();
    const w = wrap.offsetWidth || 230;
    const h = root.querySelector(".bar").offsetHeight || 30;
    let x, y;
    if (manual) {
      // stored as a fraction of the window so it survives resizing
      x = manual.fx * innerWidth;
      y = manual.fy * innerHeight;
    } else {
      const r = anchorRect(target) || rectOf(target);
      x = r.right - w + 2;
      y = r.top + (r.player ? 0 : 4); // on the YouTube player it sits on the player's top edge
    }
    const left = Math.max(0, Math.min(innerWidth - w, x));
    wrap.style.left = left + "px";
    wrap.style.top = Math.max(0, Math.min(innerHeight - h, y)) + "px";
    // a menu wider than the button slides left so it stays on screen
    if (menu.style.display === "block") {
      const over = left + menu.offsetWidth - (innerWidth - 8);
      menu.style.marginLeft = over > 0 ? -Math.min(over, left) + "px" : "0";
    }
  }

  // ───── drag the button anywhere with the mouse ─────
  const bar = root.querySelector(".bar");
  let dragging = null;
  let justDragged = false;
  bar.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    const r = wrap.getBoundingClientRect();
    dragging = { dx: e.clientX - r.left, dy: e.clientY - r.top, sx: e.clientX, sy: e.clientY, moved: false };
  });
  bar.addEventListener("pointermove", (e) => {
    if (!dragging) return;
    if (!dragging.moved && Math.hypot(e.clientX - dragging.sx, e.clientY - dragging.sy) < 4) return;
    if (!dragging.moved) bar.setPointerCapture(e.pointerId); // only now, so a plain click still reaches the buttons
    dragging.moved = true;
    bar.classList.add("drag");
    menu.style.display = "none";
    const w = wrap.offsetWidth, h = bar.offsetHeight;
    const x = Math.max(0, Math.min(innerWidth - w, e.clientX - dragging.dx));
    const y = Math.max(0, Math.min(innerHeight - h, e.clientY - dragging.dy));
    wrap.style.left = x + "px";
    wrap.style.top = y + "px";
    manual = { fx: x / innerWidth, fy: y / innerHeight };
    clearTimeout(hideTimer);
  });
  const endDrag = (e) => {
    if (!dragging) return;
    const moved = dragging.moved;
    dragging = null;
    bar.classList.remove("drag");
    try { bar.releasePointerCapture(e.pointerId); } catch {}
    if (moved) {
      justDragged = true; // swallow the click that follows the drop
      setTimeout(() => (justDragged = false), 0);
      try { chrome.storage.local.set({ dcBtnPos: manual }); } catch {}
    }
  };
  bar.addEventListener("pointerup", endDrag);
  bar.addEventListener("pointercancel", endDrag);
  bar.addEventListener("click", (e) => { if (justDragged) { e.stopImmediatePropagation(); e.preventDefault(); } }, true);
  // Start asking yt-dlp as soon as the button appears over a video (after a short pause, so a mouse
  // sweeping across a feed doesn't start a run per video). A page that is itself a video (a YouTube
  // watch page, a tweet, a reel) is asked as soon as it opens, before the mouse moves at all. The background worker keeps the answer, so
  // the list is ready - or nearly - when the button is clicked.
  let prefetchFor = null, prefetchTimer = 0;
  const prefetched = new Map(); // page url -> time asked
  function prefetch() {
    if (!target || target === prefetchFor) return;
    prefetchFor = target;
    clearTimeout(prefetchTimer);
    prefetchTimer = setTimeout(() => {
      if (!target || !target.isConnected) return;
      askNow(ytTarget());
    }, 150);
  }
  function askNow(page) {
    if (!page || Date.now() - (prefetched.get(page) || 0) < 60000) return;
    prefetched.set(page, Date.now());
    try {
      chrome.runtime.sendMessage({ type: "ytdlp", url: page }).then(noteDeps).catch(() => {});
    } catch {}
  }
  // A missing yt-dlp / Deno / ffmpeg found by the early lookup: tell the user as soon as the button is on
  // screen, without waiting for a click (once per page, so it never nags).
  let depMsg = "", depShown = false;
  function noteDeps(res) {
    depMsg = res && res.ok === false && Array.isArray(res.missing) && res.missing.length ? res.error || "" : "";
    announceDeps();
  }
  function announceDeps() {
    if (!depMsg || depShown || dismissed || wrap.style.display !== "block") return;
    depShown = true;
    toast(depMsg, 12000);
  }
  function prefetchPage() {
    if (window !== window.top) return;
    const here = dcYtPage(location.href) || (dcIsSocialSite(location.href) ? postHref({ href: location.href }) : null);
    askNow(here);
  }
  let lastHref = "";
  setInterval(() => {
    if (location.href === lastHref) return; // pages like YouTube and X change address without reloading
    lastHref = location.href;
    prefetchPage();
  }, 1000);
  function show() {
    if (dismissed) return;
    prefetch();
    wrap.style.display = "block";
    place();
    announceDeps();
    clearTimeout(hideTimer);
    const autoHide = () => {
      if (toastEl.style.display === "block") return (hideTimer = setTimeout(autoHide, 1000)); // keep the bar while a popup is up
      if (!hovering && menu.style.display !== "block") hide();
    };
    hideTimer = setTimeout(autoHide, 3000);
  }
  function hide() {
    wrap.style.display = "none";
    menu.style.display = "none";
    toastEl.style.display = "none";
  }
  function toast(t, ms = 4000) {
    toastEl.textContent = t;
    toastEl.style.display = "block";
    clearTimeout(toast.t);
    toast.t = setTimeout(() => (toastEl.style.display = "none"), ms);
  }

  document.addEventListener(
    "mousemove",
    (e) => {
      pointer = { x: e.clientX, y: e.clientY };
      if (dismissed) return;
      const v = pickVideo(e.clientX, e.clientY);
      if (v) {
        target = v;
        show();
      }
    },
    true
  );
  addEventListener("scroll", () => wrap.style.display === "block" && place(), true);
  addEventListener("resize", () => wrap.style.display === "block" && place());
  wrap.addEventListener("mouseenter", () => (hovering = true));
  wrap.addEventListener("mouseleave", () => (hovering = false));

  document.addEventListener("fullscreenchange", () => {
    const fs = document.fullscreenElement;
    if (fs && fs.tagName !== "VIDEO") fs.appendChild(host);
    else if (!fs) attach();
    else hide();
  });

  // Alt+click on a link: let the browser download it itself (the extension skips it for a few seconds)
  document.addEventListener(
    "click",
    (e) => {
      if (e.altKey) chrome.runtime.sendMessage({ type: "bypass" }).catch(() => {});
    },
    true
  );

  qBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    toast(
      "Click “Download this video” to send it to DownCraft (on YouTube, Facebook, Instagram, X and other video sites it lists the formats found with yt-dlp). Hold Alt while clicking a link to let the browser download it instead. × hides this button on the page.",
      9000
    );
  });

  xBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    dismissed = true;
    hide();
  });

  async function sendItem(it) {
    menu.style.display = "none";
    const res = await chrome.runtime.sendMessage({ type: "send", url: it.url, referer: location.href });
    if (res && res.ok) return toast("Opening DownCraft…");
    try {
      await navigator.clipboard.writeText(it.url);
      toast("DownCraft isn’t running – link copied to clipboard.");
    } catch {
      toast("DownCraft isn’t running. Start it and try again.");
    }
  }

  // ───── YouTube, Facebook, Instagram, X … (via yt-dlp in the desktop app) ─────
  // These players use blob: video, so nothing can be sniffed there; the app lists the formats instead.
  // On a feed the page address says nothing about the video, so the link of the post around the
  // hovered video is used.
  const POST_PATHS = [
    /^\/[^/]+\/status\/\d+/, // x.com, twitter.com
    /^\/(?:[^/]+\/)?(?:p|reels?|tv)\/[\w-]+/, // instagram
    /^\/reel\/\d+/, // facebook
    /^\/[^/]+\/(?:videos|posts)\/[\w.-]+/,
    /^\/videos\/\d+/,
    /^\/share\/[vrp]\/[\w-]+/,
    /^\/@[^/]+\/video\/\d+/, // tiktok
  ];
  function postHref(a) {
    try {
      const u = new URL(a.href, location.href);
      if (u.hostname !== location.hostname || !dcIsSocialSite(u.href)) return null;
      if (u.pathname.replace(/\/$/, "") === "/watch" && u.searchParams.get("v")) return u.origin + "/watch/?v=" + u.searchParams.get("v");
      const re = POST_PATHS.find((r) => r.test(u.pathname));
      return re ? u.origin + u.pathname.match(re)[0] : null;
    } catch {
      return null;
    }
  }
  // address to give yt-dlp for the video being hovered, or null when this page isn't one for yt-dlp
  function ytTarget() {
    const yt = dcYtPage(location.href);
    if (yt) return yt;
    if (!dcIsSocialSite(location.href)) return null;
    let el = target;
    for (let i = 0; el && el !== document.body && i < 14; i++, el = el.parentElement) {
      for (const a of el.querySelectorAll ? el.querySelectorAll("a[href]") : []) {
        const h = postHref(a);
        if (h) return h;
      }
    }
    return dcVideoPage(location.href);
  }

  const fmtDur = (s) => {
    s = Math.round(s || 0);
    const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = s % 60;
    return (h ? h + ":" + String(m).padStart(2, "0") : m) + ":" + String(r).padStart(2, "0");
  };
  const fmtLabel = (f) => (f.kind === "audio" ? "Audio" + (f.lang ? " (" + f.lang + ")" : "") : f.height ? f.height + "p" + (f.fps > 30 ? f.fps : "") : f.note || "Video");
  const fmtDetail = (f) =>
    [f.ext.toUpperCase(), f.codec, f.kind === "audio" && f.tbr ? Math.round(f.tbr) + "k" : "", f.hdr ? "HDR" : "", f.hls ? "HLS" : ""].filter(Boolean).join(" · ");
  const addRow = (cls, text) => {
    const d = document.createElement("div");
    d.className = cls;
    d.textContent = text;
    menu.appendChild(d);
    return d;
  };

  async function sendFormat(res, f, a) {
    menu.style.display = "none";
    let r = null;
    try {
      r = await chrome.runtime.sendMessage({ type: "send-format", format: f, audio: a || null, title: res.title, uploader: res.uploader, pageUrl: location.href });
    } catch {}
    if (!(r && r.ok)) return toast("DownCraft isn’t running. Start it and try again.");
    toast("Opening DownCraft…");
  }

  function renderFormats(res) {
    menu.innerHTML = "";
    const entries = dcEntries(res);
    let n = 0;
    entries.forEach((ent, k) => {
      const title = ent.title || ent.uploader || "Video";
      const rows = dcChoices(ent); // the sound of video-only rows is merged in by the app
      if (entries.length > 1) addRow("msg", "Video " + (k + 1) + " of " + entries.length + (ent.duration ? " · " + fmtDur(ent.duration) : ""));
      rows.forEach((c, i) => {
        if (c.kind === "audio" && (i === 0 || rows[i - 1].kind !== "audio")) addRow("sep", "");
        const d = document.createElement("div");
        d.className = "row";
        const no = document.createElement("span");
        no.className = "no";
        no.textContent = ++n + ".";
        const tt = document.createElement("span");
        tt.className = "tt";
        tt.textContent = title;
        const inf = document.createElement("span");
        inf.className = "inf";
        inf.textContent = c.text;
        d.append(no, tt, inf);
        if (c.size) {
          const sz = document.createElement("small");
          sz.textContent = (c.approx ? "~" : "") + fmtSize(c.size);
          d.append(sz);
        }
        d.title = c.tip;
        d.addEventListener("click", () => sendFormat(ent, c.f, c.a));
        menu.appendChild(d);
      });
    });
    if (!n) return addRow("msg", "No downloadable formats found for this video.");
    menu.style.display = "block";
    place();
  }

  async function showYtdlp(url, force) {
    menu.innerHTML = "";
    addRow("msg", "Getting the format list from yt-dlp…");
    menu.style.display = "block";
    let res = null;
    try {
      res = await chrome.runtime.sendMessage({ type: "ytdlp", url, force: !!force });
    } catch {}
    if (menu.style.display !== "block") return; // closed while waiting
    if (!res || !res.ok) {
      menu.style.display = "none";
      // a missing yt-dlp / Deno / ffmpeg: the app says what to install - leave that on screen long enough to read
      const missing = res && Array.isArray(res.missing) && res.missing.length;
      return toast((res && res.error) || "Couldn’t reach DownCraft. Start it and try again.", missing ? 12000 : 8000);
    }
    renderFormats(res);
  }

  dl.addEventListener("click", async (e) => {
    e.stopPropagation();
    if (justDragged) return;
    if (menu.style.display === "block") return (menu.style.display = "none");
    const page = ytTarget();
    if (page) return showYtdlp(page);
    let items = [];
    const direct = target ? target.currentSrc || target.src || "" : "";
    if (/^https?:/i.test(direct)) items.push({ url: direct, size: 0, stream: /\.(m3u8|mpd)(\?|$)/i.test(direct) });
    let res = null;
    try {
      res = await chrome.runtime.sendMessage({ type: "list" });
    } catch {}
    for (const i of (res && res.items) || []) if (!items.some((x) => x.url === i.url)) items.push(i);
    // HLS (.m3u8) is supported; MPEG-DASH (.mpd) is not.
    const ok = items.filter((i) => !/\.mpd(\?|$)/i.test(i.url));
    if (!ok.length) {
      // nothing to sniff (or only MPEG-DASH): yt-dlp knows well over a thousand sites, so let it have a go
      if (/^https?:/i.test(location.href)) return showYtdlp(location.href.split("#")[0], true);
      return toast(
        items.length
          ? "This video uses MPEG-DASH, which DownCraft can’t download yet."
          : "No video link found yet. Let the video play for a few seconds, then try again.",
        6000
      );
    }
    if (ok.length === 1) return sendItem(ok[0]);
    menu.innerHTML = "";
    for (const it of ok) {
      const d = document.createElement("div");
      d.textContent = nameOf(it.url);
      const s = document.createElement("small");
      s.textContent = it.stream ? "HLS stream" : fmtSize(it.size);
      d.appendChild(s);
      d.title = it.url;
      d.addEventListener("click", () => sendItem(it));
      menu.appendChild(d);
    }
    menu.style.display = "block";
  });
})();
