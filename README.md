<p align="center"><img src="app-icon.png" alt="PROVIEW CCTV" width="96"></p>

# proview-cctv-web

PROVIEW CCTV cameras on a desktop instead of a phone. A Rust server and a Tauri
desktop app that speak to the vendor cloud the way the PROVIEW Android app
(`com.proview.cctv`, a Xiongmai / XMEye white-label) does: sign in with your
PROVIEW account and watch, move, play back and configure your cameras. No camera
password, no port forwarding, no vendor binary.

Current release: **v0.5.0** ([CHANGELOG.md](CHANGELOG.md)). Not affiliated with
the vendor; it talks only to cameras and an account you own.

## Features

| Area | What you get |
|---|---|
| Live view | One camera fills the window, or a grid of up to nine tiles; main or sub stream; dual-lens picture in picture or split; fisheye dewarp; snapshot; keyboard shortcuts (`?`) |
| Sound and Talk | Camera sound in live view and playback; Talk sends your microphone to the camera |
| PTZ | Move pad on the picture with Slow / Normal / Fast that follows the camera's mirror; press and hold, tap-to-aim on the wide lens, presets and tours |
| Playback | Ruler timeline inside Live with recordings and event markers, 24 h to 10 min zoom, calendar, thumbnails, speed 1/2x to 8x, MP4 download |
| Device settings | Encode, network, storage and SD card, record schedule, OSD, alarm and detection, image, light and sound, time, language, users, restart |
| Events | Side panel and page with pictures, filter chips, day picker, delete and clear all; opening an event plays from 5 s before it; native desktop notifications and a tray icon |
| Account | Profile, password change and reset, register, sharing with permissions and by QR, cloud storage plan |
| Without the cloud | Live view and controls of a camera on your LAN (RTSP, ONVIF, DVRIP); built-in sample data for a demo |

Every feature of the phone app, with its state here and the boundaries that
remain: [docs/parity.md](docs/parity.md).

## Install

Download from the [latest release](https://github.com/tickernelz/proview-cctv-web/releases/latest);
`SHA256SUMS` covers every file.

| Platform | Desktop app | Server binary |
|---|---|---|
| Windows x64 / arm64 | `.msi` or `-setup.exe` | `proview-server-<arch>-pc-windows-msvc.zip` |
| macOS Apple silicon / Intel / universal | `.dmg` | `proview-server-<arch>-apple-darwin.tar.gz` |
| Linux x86_64 / aarch64 | `.AppImage`, `.deb`, `.rpm` | `proview-server-<arch>-unknown-linux-{gnu,musl}.tar.gz` |

The installers are not code-signed. On Windows choose SmartScreen → More info →
Run anyway; on macOS right-click → Open, or System Settings → Privacy & Security → Open
Anyway on macOS 15 and later (or `xattr -dr com.apple.quarantine`);
on Linux `chmod +x` the AppImage. Per-platform detail:
[docs/install.md](docs/install.md). Running the server behind TLS with systemd:
[docs/deploy.md](docs/deploy.md).

## Quick start

**Desktop app.** Open it and sign in with the email and password of your
PROVIEW account. Your cameras appear; pick one.

**Server.** It serves the same page on `http://127.0.0.1:8787`:

```sh
export ROCKET_SECRET_KEY="$(openssl rand -base64 32)"
export PROVIEW_DATA_DIR="$HOME/.local/share/proview"
./proview-server            # --listen 0.0.0.0:8787 to bind elsewhere
```

A release build refuses to start without `ROCKET_SECRET_KEY` (it signs the
sign-in cookie; keep it stable so sessions survive a restart).
`PROVIEW_DATA_DIR` is where Settings are saved; without it the server keeps
them in memory only.

**Demo, no camera and no account.** `PROVIEW_PROVIDER=fixture` serves recorded
sample data; sign in with `fixture@example.invalid` / `fixture-password`.

## Configuration

Everything is in the app: **Settings → App → Configuration** (the sign-in
screen's **Open settings** button goes straight there). Settings answer only a browser
on the same machine as the server (a loopback address); for a remote server use
an SSH tunnel or the environment. Values are saved to `settings.json` in the data folder; a key
read only at startup carries a **Restart** tag, and the desktop app offers a
**Restart the app** button for it. Environment variables still work for
unattended installs; a variable set when the process starts wins over the
saved value.

The essentials:

| Variable | Default | Meaning |
|---|---|---|
| `ROCKET_SECRET_KEY` | generated in debug builds | server only: signs the session cookie; required by a release build |
| `PROVIEW_LISTEN` | `127.0.0.1:8787` | server only: bind address as IP and port; `--listen` wins over the environment, which wins over a loopback value saved in Settings (a saved network address is ignored) |
| `PROVIEW_DATA_DIR` | unset | server only: where `settings.json` and `mode.json` live (the desktop app always uses its own app data folder) |
| `PROVIEW_PROVIDER` | the vendor cloud | `fixture` for sample data |
| `PROVIEW_DEVICE_RTSP`, `_HOST`, `_ONVIF`, `_USER`, `_PASS` | unset | a camera on the LAN (video, DVRIP, ONVIF, credentials) |
| `PROVIEW_FFMPEG` | `ffmpeg` | ffmpeg program for the H.265 fallback |

Every key, its label, the precedence rule and the API:
[docs/settings.md](docs/settings.md). LAN, cloud and fixture providers:
[docs/providers.md](docs/providers.md).

The browser never sees a vendor token: it holds only an opaque `pvw_session`
cookie, and every vendor call happens server side.

## ffmpeg (fallback only)

Nothing else needs installing for live view, playback and downloads. Where the
webview decodes H.265 (Windows WebView2 and macOS WKWebView where the OS carries
the HEVC decoder), the camera's H.265 is muxed into fMP4 in process, and H.264
is remuxed in process everywhere. `ffmpeg` (from `PATH`, or the "ffmpeg
program" setting) is used only when it is there, for:

- transcoding H.265 to H.264 for a webview that cannot decode H.265, or for a
  stream that reorders frames (once per stream, shared by every viewer);
- the thumbnail of a recorded clip that holds no motion picture;
- turning cloud-storage clips into MP4.

Without it those three show an error and everything else keeps working. Which
webview decodes what: [docs/webview-matrix.md](docs/webview-matrix.md).

## Development

Requirements: Rust stable, Node 22 (Vite needs 20.19+ or 22.12+), and for the
desktop app the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
(cd apps/web && npm ci && npm run build)    # the server embeds apps/web/dist
cargo run -p proview-server                 # http://127.0.0.1:8787
PROVIEW_PROVIDER=fixture cargo run -p proview-server
```

Hot reload: keep the server on 8787, run `npm run dev` in `apps/web` and open
`http://localhost:5173` (Vite proxies `/api`, `/ws` and `/media`). Desktop
shell (`cargo install tauri-cli --version "^2.0"`): `cargo tauri dev`; with
`PROVIEW_WEB_DEV=1` (debug builds) it opens the window on Vite instead of the
embedded page.

Tests, as CI runs them:

```sh
cargo fmt --all --check
cargo clippy -p proview-core -p proview-server --all-targets -- -D warnings
cargo test -p proview-core -p proview-server
node --test tests/web/*.test.mjs            # needs a built proview-server and Chrome
(cd e2e && npm ci && npx playwright test)   # see e2e/README.md
```

| Path | What |
|---|---|
| `crates/core` | protocol and media: cloud envelope, relay, DVRIP, RTSP, ONVIF, fMP4, providers |
| `crates/server` | Rocket app: JSON API, WebSocket streams, settings store, sessions |
| `src-tauri` | desktop shell: embedded server, tray, notifications, restart |
| `apps/web` | UI: Vue 3, TypeScript, Vite, Tailwind |
| `tests/web`, `e2e` | UI tests in headless Chrome and Playwright |
| `fixtures` | recorded device data for the fixture provider |
| `tools` | measurement and acceptance scripts |

## Documentation

Start at [docs/README.md](docs/README.md). Most used:
[install](docs/install.md), [deploy](docs/deploy.md),
[settings](docs/settings.md), [parity](docs/parity.md),
[protocol record](docs/protocol.md), [performance](docs/performance.md).

## License

MIT, see [LICENSE](LICENSE). This project ships no vendor code, APK, `.so` or
extracted keys; release builds carry the app's platform identity from
repository secrets, and a build from source brings its own
(`PROVIEW_APP_UUID`, `PROVIEW_APP_KEY`, `PROVIEW_APP_SECRET`) or runs with the
cloud sign-in off.
