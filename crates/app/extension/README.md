# DownCraft browser extension (Chrome / Edge / Brave / Opera — Manifest V3)

Shows a **Download this video** button (with ? help and × hide) in the top-right corner of any video you hover, and lists
every direct media link a page loads (toolbar icon). Clicking sends the link — plus Referer, User-Agent
and cookies so protected links work — to the desktop app, which opens *Download File Info*.

## Install (developer mode)
**Easiest:** in DownCraft open the **Extension** menu → *Browser extension…*, then drag the logo onto
`chrome://extensions` (Developer mode on).

**Manually:**
1. Start **DownCraft**.
2. Open `chrome://extensions` → enable **Developer mode** → **Load unpacked** → choose this `extension` folder.
3. Open a page with a video, hover it, press the button.

## How it talks to the app
`POST http://127.0.0.1:19870/add` (loopback only). The app accepts requests only from
`chrome-extension://` origins, so websites can't use it, and nothing downloads until you confirm.
If the app isn't running the link is copied to your clipboard instead.

## Limits
* Detects direct files (mp4, webm, mkv, mp3 …) and **HLS streams (`.m3u8`)**. `blob:` videos are covered by the sniffed list.
* HLS: best quality is chosen, AES-128 encrypted streams are decrypted, the result is one `.ts` (or `.mp4` for fMP4) file.
* MPEG-DASH (`.mpd`), live streams and DRM-protected HLS (SAMPLE-AES / Widevine / FairPlay) are not supported.
* Streams that keep audio in a separate playlist download without that audio track.
* DRM-protected video (Netflix, etc.) can't be downloaded; this extension doesn't try.

## Video sites via yt-dlp (0.3, widened in 0.5)
YouTube, Facebook, Instagram, X / Twitter, TikTok, Vimeo, Twitch, Reddit, Dailymotion and similar sites play through `blob:`
video or split streams, so there is no media link to sniff. On these pages the button (and the toolbar popup) therefore
asks the desktop app to run **yt-dlp** and lists what it finds, grouped as *Video + audio*,
*Video only* and *Audio only*, with codec and size. Pick one and it goes through the normal *Download File Info* dialog.

* Needs **yt-dlp**. In DownCraft open **Tasks → Tools (yt-dlp, ffmpeg)…** and press *Install* - it downloads the official
  build from GitHub into DownCraft's own `tools` folder (and *Update* / *Remove* manage it later). The same window installs
  **Deno** (recent yt-dlp needs a JavaScript runtime to read YouTube - install it if the list comes back empty) and
  **ffmpeg** (joining video and audio; the button asks for it too). Copies you already have on `PATH` or next to the DownCraft executable
  are found and used too, and `DOWNCRAFT_YTDLP` can point at a specific yt-dlp.
* **Missing tools:** if yt-dlp, **Deno** (YouTube pages only) or **ffmpeg** isn't installed, the *Download this video* button shows a popup
  naming what is missing (e.g. *“yt-dlp and ffmpeg aren't installed. In DownCraft open Tasks → Tools and press Get next to yt-dlp and ffmpeg.”*)
  instead of a format list. It also appears by itself, once per page, as soon as the button shows over a video, and DownCraft opens its **Tools** window so you can press *Get* right away (at most once a minute, so closing it sticks). Install them there and click the button again - nothing needs restarting. The bridge answers
  `{ "ok": false, "missing": ["Deno"], "error": "…" }` in that case.
* Only the current video is listed (playlist / Mix parameters are ignored). A post with several videos (an X thread, an
  Instagram carousel) lists each one. Live streams are not supported.
* In a feed (X timeline, Instagram, Facebook) the link of the post around the video you hover is used, not the feed's address.
* Posts that need a sign-in (private accounts, some Instagram / Facebook videos): the extension sends the browser's cookies for
  that page with the request (YouTube's are never sent). The app writes them to a temporary file for yt-dlp and deletes it
  right after.
* On any other page where nothing could be sniffed, the button and the popup fall back to yt-dlp, which knows well over a thousand sites.
* The chosen link is downloaded by DownCraft's own engine (segments, resume, speed limit), with yt-dlp's request headers.
* **Video only** formats (1080p and above on YouTube) have no sound - download an audio track as well and merge them
  (e.g. with ffmpeg). *Video + audio* formats are complete files, but YouTube often offers them only up to 360p.
* The bridge endpoint is `POST /ytdlp` `{ "url": "<page url>", "cookies": [ … ] }` (`cookies` optional); like `/add` it only answers `chrome-extension://` origins.

## Download interception (0.2)
Downloads the browser starts are cancelled and handed to DownCraft (you still confirm in *Download File Info*).
* **Alt+click** a link to let the browser download it itself.
* If DownCraft isn't running, the browser keeps the download. `blob:` / `data:` / `file:` downloads always stay with the browser.
* Right-click a link, video or audio → **Download with DownCraft**.
* Filters (file types, minimum size, excluded sites): extension **Options**.
