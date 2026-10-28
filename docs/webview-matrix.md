# Video support per webview

The app decodes H.264 in fragmented MP4 delivered over a binary WebSocket into
MediaSource, and offers a second transport that hands HEVC access units to
WebCodecs when the webview has it. H.265 is detected at runtime and reported,
never assumed. Recorded clips still travel over HTTP.

| Webview | Platform | H.264 fMP4 | H.265 | Notes |
|---|---|---|---|---|
| WebView2 (Chromium) | Windows 10/11 | yes | yes, if the OS has the HEVC codec extension | hardware decode via Media Foundation |
| WKWebView | macOS 13.3+ | yes | yes on Apple silicon for most profiles | hardware decode via VideoToolbox |
| WebKitGTK | Linux | yes, when a GStreamer H.264 decoder is installed (`gstreamer1.0-libav`, or `gstreamer1-plugin-openh264` on Fedora) | not guaranteed | distro dependent; the app surfaces a clear message instead of a black frame |
| Chromium / Edge | any | yes | depends on build | WebCodecs available |
| Firefox | any | yes via MSE | depends on build | no WebCodecs for H.265 |

Measured verification so far: Chromium (headless 129, Linux) decoded the served
fMP4 — `videoWidth=640`, `currentTime` advancing, `error=null`.

WebKitGTK 2.54.1 (Fedora 44, `webkit2gtk4.1`, GStreamer 1.28 with
`plugins-good` and `plugins-bad-free` but no `gstreamer1-libav` and no
`gstreamer1-plugin-openh264`), 2026-10-27: the desktop app (`proview-desktop`
0.2.5 debug, v0.3.0 UI, under Xvfb) and MiniBrowser both rendered the UI, but
`MediaSource.isTypeSupported('video/mp4; codecs="avc1.42E01E"')` was `false`,
so the stage showed "This browser cannot decode the stream codec" instead of a
picture. On Fedora install `gstreamer1-plugin-openh264` or
`gstreamer1-plugin-libav`; Debian/Ubuntu `gstreamer1.0-libav`. The same run
reported `showPopover` and `color-mix()` support.

WebView2 and WKWebView have **not** been exercised yet; that is tracked in
`docs/parity.md` and must be re-checked on each release runner.

## Page floor (CSS and JS)

The page is built with Tailwind CSS v4 (`@tailwindcss/vite`), whose output
needs cascade layers, `@property` and `color-mix()`. Its documented floor is
Chrome 111, Safari 16.4 and Firefox 128
(https://tailwindcss.com/docs/compatibility), so that is the page's floor:
Chrome/Edge 111, Firefox 128, Safari/WKWebView 16.4. `apps/web/vite.config.ts`
pins `build.target` (scripts) and `build.cssTarget` (Lightning CSS) to it, so the
build may emit syntax up to that floor (for example `@media (width <= 640px)`)
and nothing newer. Raising the floor means changing both places.

The desktop shell enforces it on macOS: WKWebView is the system WebKit, and
macOS 13.3 is the first release that ships Safari 16.4, so
`bundle.macOS.minimumSystemVersion` is `13.3` in `src-tauri/tauri.conf.json`.
Older macOS with Safari updated to 16.4 would render the page, but the installer
cannot check the Safari version. Windows (WebView2, evergreen Chromium) and
Linux (WebKitGTK 2.50 on Ubuntu 22.04 and newer) are above the floor. In server
mode an older browser gets an unstyled or partly styled page.

## Why fMP4 and not HLS first

HLS adds a segmenter, a playlist server and a 1–3 segment latency floor. The
live path is a binary WebSocket carrying fMP4 fragments so the browser can feed
`MediaSource` (or `VideoDecoder` where available) with the smallest possible
delay. HLS stays available only for the vendor's own cloud restream.
