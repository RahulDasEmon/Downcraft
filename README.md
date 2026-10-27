<p align="center"><img src="assets/logo-256.png" width="120" alt="DownCraft"></p>

# DownCraft

A free, open-source, multi-connection **download manager in pure Rust** — the same layout and
workflow as Internet Download Manager, running natively on **Windows, macOS and Linux**.

## Features

- Segmented downloads: up to 32 parallel connections per file, byte-exact reassembly
- **Dynamic segmentation** (from FluxDown): idle connections split the slowest piece in half, so one slow connection never drags out the finish
- **Adaptive connections**: a server that answers extra connections with 429/403/503 gets fewer of them, remembered per host for 24 h
- Dead-connection detection (30 s without data → reconnect), instant pause, integrity check that the pieces tile the file
- Resume validation: ETag / size are re-checked, and a file that changed on the server restarts instead of being corrupted
- Global + per-download speed limits (shared pacing), HTTP / SOCKS5 / system proxy, automatic retry with back-off, disk-space check
- Add several URLs at once (one per line)
- **Video sites via yt-dlp**: paste a Facebook, Instagram, X / Twitter, TikTok, YouTube, Vimeo, Reddit … post into *Add URL* (or press *As video* for any other site yt-dlp knows) and pick a quality; posts with several videos list each one. Picture and sound that come as two files are downloaded by the normal engine and joined with ffmpeg. Posts that need a sign-in work through the browser extension's cookies or *Options → Downloads → Video sites*
- **HLS (`.m3u8`) streams**: best quality picked, AES-128 decrypted, `EXT-X-BYTERANGE` supported, parallel segments, resumable, saved as one `.ts`/`.mp4`
- Pause / resume across restarts (state in `downloads.json`), automatic retry with back-off
- Falls back to a single connection when the server has no `Range` support
- File name from `Content-Disposition` / URL, automatic category (Compressed, Documents, Music, Programs, Video)
- Per-category folders, "remember this path", per-download speed limiter
- Main download queue, Start/Stop queue, Scheduler
- IDM-style UI: toolbar, category tree, sortable-looking list, *Download File Info*,
  live *progress with per-connection map*, and *Download complete* popups
- **Toolbar skins**: *Options → Toolbar icons → Load skin (.zip)…* takes an IDM-style skin pack (a `.tbi` plus
  its normal and "hot" `.bmp` strips of 12 icons; 32-bit alpha, or 24-bit with magenta as the transparent colour).
  It is remembered across restarts; *Reset* restores the built-in icons
- **Options** (tabbed): clipboard monitor with file-type filter, start at login / minimized, action when the queue ends (close, sleep, shut down),
  per-category folders, per-server connection limits, download quota, site logins (HTTP Basic), proxy with credentials and bypass list,
  IPv4/IPv6 and outgoing-address choice, timeouts, duplicate-link handling, a command to run after each download (e.g. a virus scan), event sounds
- Portable mode: create a `downcraft-data/` folder next to the executable

## Browser extension

`crates/app/extension/` is a Manifest V3 extension (Chrome, Edge, Brave, Opera). It takes over browser downloads (Alt+click to bypass), adds a right-click *Download with DownCraft*, and adds a **Download this video**
button to the corner of videos and sends direct links (with Referer + cookies) to the app through a
loopback-only endpoint (`127.0.0.1:19870`). On YouTube, Facebook, Instagram, X and other video-site pages the button lists the formats found by yt-dlp; install yt-dlp, ffmpeg and Deno from **Tasks → Tools**. On start-up, while the browser extension isn't connected or one of those helpers is missing, a **Set up DownCraft** window offers both (drag the logo onto `chrome://extensions` to install the extension, a *Get* button per helper, *Get all*, and *Don't show again*); reopen it any time from **Tasks → Setup guide…**. See `crates/app/extension/README.md`.

## Build & run

```bash
cargo run --release -p downcraft-app       # needs Rust 1.82+
cargo test -p downcraft-core               # engine tests (local Range server)
cargo test -p downcraft-app                # toolbar-skin importer tests
```

Linux build deps: `libxkbcommon-dev libwayland-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.

## Layout

```
crates/core   download engine (no GUI): probe, segments, resume, queue, persistence
crates/app    egui/eframe desktop UI  → binary `downcraft`
assets/       logo
```

## Roadmap

MPEG-DASH, FTP, separate HLS audio tracks, custom categories, site grabber,
proxy auto-config (PAC) scripts, a separate temporary folder for part files.

## Legal

DownCraft is an independent project, not affiliated with Tonec Inc. It contains no IDM code or assets.
MIT licensed.
