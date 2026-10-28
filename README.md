# proview-cctv-web

A small Rocket server, and a Tauri desktop shell around it, that put PROVIEW
CCTV (package `com.proview.cctv`, a Xiongmai / XMEye white-label) cameras on a
desktop instead of a phone.

Current release: **v0.3.5** (`CHANGELOG.md` carries what changed). Status: a
working server and desktop app with configuration in the app itself. Live video
from a LAN camera works end to end. The vendor cloud account login **ships
here**: email + password is sealed in the signed v3 envelope from Rust
(`crates/core/src/cloud/envelope.rs`) and the account's device list follows it
(see "Cloud account login" below). `docs/protocol.md` records exactly what is
proven and what is not.

## What works today

| Piece | State |
|---|---|
| Rocket server, static UI, JSON API | works |
| Server-side session (private cookie → in-memory vendor session) | works |
| Settings view: every server key, from the sign-in screen, saved to `settings.json` | works, saves merge, secrets never read back |
| Desktop app (Tauri 2): Settings, mode choice, "Restart the app" | built by CI for 7 targets; launched on Linux (WebKitGTK 2.54, Xvfb) for v0.3.0, signs in and renders; live H.264 there needs the GStreamer H.264 decoder (`docs/webview-matrix.md`) |
| Desktop alarm notifications and tray | works from the desktop process: one native system notification per new alarm (keyed by device, channel and id; survives a restart; one summary after a gap), tray with Open / Live / Quit, a single instance, close-to-tray on and minimize-to-tray off by default, all four switches in Settings; `docs/desktop-notifications.md` states what a headless box could and could not measure |
| Web UI (v0.3.0): one camera fills the window, camera menu, lens menu, every-camera view, move pad, keyboard shortcuts (`?`), Settings as a screen | works; `DESIGN.md` |
| DVRIP framing, login handshake, session | works, tested against the camera at `192.168.0.100` (`Ret: 100`) and a mock |
| Annex-B NAL splitter, H.264/H.265 detection, SPS parse | works, tested |
| Media packaging to fMP4 (playable) | works: Annex-B clip → init+moof+mdat, verified with ffprobe and in-browser playback |
| Device caps from the app cloud | works: `GET /api/cloud/device-caps?sn=<serial>` answers model, firmware, WAN address and the capability block, no account session needed; unknown serial is a 404 |
| Live video to the browser | works: RTSP pull + H.265 depacketize, then either one transcode per stream (fMP4 over WebSocket into MSE) or HEVC passthrough into WebCodecs |
| Dual-lens split of one stream | works: `GET /api/lens` returns the lens crops for a stacked frame; `docs/contracts/lens.md` |
| Provider seam (cloud / local / fixture) | works: the same UI over the vendor cloud, a camera on the LAN, or recorded sample data |
| Fixture provider (no camera, no account) | works, opt-in (`PROVIEW_PROVIDER=fixture`); the default provider is the vendor cloud |
| Vendor cloud account login | works: `POST /api/login` seals email + password in the signed v3 envelope from Rust and answers an `accessToken`; no vendor binary is involved |
| Device list from the vendor cloud | works: `GET /api/devices` returns the account's devices after that login (`msdlistCode`); `401` until signed in |

The server never sends a vendor token to the browser: the browser holds only an
opaque `pvw_session` cookie, and every vendor call happens server side.

## Run

```sh
ROCKET_SECRET_KEY="$(openssl rand -base64 32)" cargo run --release -p proview-server
# http://127.0.0.1:8787
```

The release build refuses to start without `ROCKET_SECRET_KEY` (it signs the
sign-in cookie, and Rocket requires a real key outside debug builds). Any value
of 32 random bytes in base64 or 64 hex characters works; make it stable so
sessions survive a restart. A debug build generates its own.

Open Settings from the sign-in screen (the link under the form) and fill in what
you need; nothing has to be exported by hand. Sample data, no camera needed:

```sh
PROVIEW_PROVIDER=fixture cargo run -p proview-server
```

A debug build generates its own cookie secret, so the demo needs no environment
at all.

```
email:    fixture@example.invalid
password: fixture-password
```

Configuration lives in the app. Values are written to `settings.json` in the
data directory, each save merges into what is already stored, and a key that is
read once at startup is marked "needs restart" in the view — the desktop build
offers a **Restart the app** button for those and restarts the shell through
`POST /api/desktop/restart` with the per-run token the served page carries (no
IPC, which the loopback origin is refused; the route and the page take only the
bound `127.0.0.1:<port>` authority), a server says to restart the process. Keys
read per request apply as soon as they are saved.
`docs/settings.md` lists every key, the precedence rule and the API.

## Cloud account login: the signed v3 envelope, in this repository

The vendor app logs in with email + password through `libFunSDK.so`
(`XM_Video_Login_XMAccount`), not through the v2 portal this repository talks
to. That library cannot be redistributed, so the envelope was read out of it on
2026-10-27 and reimplemented in Rust: no vendor binary, chroot or emulator is
needed at run time, and none of it is in this repository or in a release. What
the login does:

- email + password → an `accessToken` (JWT, HS256, 30 days);
- the platform exchange on `boss22-api.xmcsrv.net` answers `code 2000` with a
  device token;
- signed calls go to `<host>/v3/<tsid>/<sig>.rs` with `tsid` = `0000003` plus
  epoch milliseconds and a 32-hex `sig`; the body is 96 lowercase hex
  characters, i.e. API-encrypted 28 bytes (12 random + 4-byte timestamp +
  12-char seed), and that path needs no `UniqueCode` header;
- the device list follows on `msdlistCode` and carries the account's devices.

`POST /api/login` with `{"email","password"}` runs those steps and answers
`{"ok":true,...}`; `GET /api/devices` then lists the devices and `GET /api/me`
reports the account. The app's platform identity ships inside the app: the
`release` workflow bakes `PROVIEW_APP_UUID`, `PROVIEW_APP_KEY` and
`PROVIEW_APP_SECRET` into the binary from repository secrets, so signing in asks
for the email and the password and nothing else. The environment still wins per
value when it is set, which is the way a build made from source brings its own
identity; a build without one answers that the vendor cloud login is off instead
of asking anyone to fill in variables. Rotating the identity is a release, not a
setting: a binary keeps the identity it was built with, so the vendor has to
issue a new one and every copy needs a build made after that. The product path measured 0.30–0.39 s for sign-in plus device list,
against 65 s for the out-of-tree emulator the derivation used
(`docs/performance.md`). `docs/protocol.md` sections 3A and 3B carry the
derivation and the shipped implementation.

## Live view from a LAN camera

The local provider needs no cloud account: point the server at the camera and it
pulls, depacketizes and either transcodes once per channel or forwards HEVC to a
WebCodecs client, then fans the fragments out over a WebSocket.

```sh
PROVIEW_DEVICE_RTSP=192.168.0.100:554 \
PROVIEW_DEVICE_HOST=192.168.0.100:34567 \
PROVIEW_DEVICE_ONVIF=192.168.0.100:8899 \
PROVIEW_DEVICE_USER=admin \
PROVIEW_DEVICE_PASS='the device password' \
cargo run --release -p proview-server
```

The same five values can be typed into Settings → Configuration → Local network
and saved; the difference is that the environment wins over the file until you
clear it in the view.

Device control (ONVIF for movement and presets, DVRIP for settings and
snapshots) is served under `/api/device/controls`: the capability report lists
every feature per lens with the device wording that backs it, `ptz` moves the
lower lens with an explicit timeout and still sends the stop on a fresh
connection when the start misses that timeout, and `snapshot`, `light`, `flip` and
`record` read or write the DVRIP settings. `GET /api/device/controls/settings`
returns the read-only ONVIF device, encoder, imaging and network view.

`ffmpeg` must be on `PATH` for HEVC cameras (or point the "ffmpeg program"
setting at the binary). The camera in hand delivers H.265 only, so the server
transcodes to H.264 baseline once per (device, channel, stream) and every viewer
shares the result; browsers do not have to decode HEVC. Measured latency and
memory are in `docs/performance.md`, the protocol in `docs/protocol.md`
section 8. The stream source stays warm for 60 s after the last viewer leaves,
so re-opening a camera is a warm start (~200 ms) instead of a cold one
(~2.5-5 s, bounded by the camera's keyframe interval).

## Configuration

Settings → Configuration is the first place to look. Every key in the file is
readable there, with a plain label, a hint and a "needs restart" marker; secrets
are written but never shown back. The environment remains supported for
unattended installs and wins over the file until the key is cleared in the view.

| Variable | Settings label | Default | Meaning |
|---|---|---|---|
| `PROVIEW_PROVIDER` | Provider | `proview` | `proview`, `local` or `fixture` |
| `PROVIEW_MODE` | Default mode | `cloud` | `cloud` or `local`; bootstrap default for `GET /api/config/mode` |
| `PROVIEW_CLOUD_BASE` | Cloud address | `https://pub-token.xmeye.net` | vendor cloud base |
| `PROVIEW_PORTAL_BASE` | Portal address | vendor default | host used by the account login |
| `PROVIEW_CAPS_BASE` | Capability address | `https://caps.xmcsrv.net` | host that answers device capabilities by serial |
| `PROVIEW_CLOUD_SESSION_TTL` | Cloud session lifetime | vendor default | seconds a cloud session is kept |
| `PROVIEW_DEVICE_HOST` | Camera address | unset | `host:port` for DVRIP (probe, settings, snapshots) |
| `PROVIEW_DEVICE_RTSP` | Camera video address | unset | `host:port` for RTSP; what `local` mode streams |
| `PROVIEW_DEVICE_ONVIF` | Camera control address | unset | ONVIF base for movement and presets |
| `PROVIEW_DEVICE_SERIAL` | Camera serial for the device sign-in | unset | the cloud path sends `PROVIEW_DEVICE_USER`/`PASS` only to this serial |
| `PROVIEW_DEVICE_USER` | Camera user | `admin` | device-side account |
| `PROVIEW_DEVICE_PASS` | Camera password | empty | device-side password, secret |
| `PROVIEW_SESSION_TTL` | Sign-in lifetime | `3600` | browser session lifetime, seconds |
| `PROVIEW_LISTEN` | Server address | `127.0.0.1:8787` | standalone server bind address |
| `PROVIEW_FFMPEG` | ffmpeg program | `ffmpeg` | binary used for the HEVC → H.264 bridge |
| `PROVIEW_LENS_PTZ` | Movement channel | measured default | lens index that pans/tilts (bottom lens on the dual-lens IPC) |
| `PROVIEW_LENS_CAPABILITIES` | Lens features | unset | comma list of `SystemFunctionBean$OtherFunction` flags to advertise |
| `PROVIEW_FIXTURE_DIR` | Sample data folder | `fixtures` | directory the fixture provider reads |
| `PROVIEW_PROFILE` | Credential profile | `default` | keychain profile name for the desktop |
| `PROVIEW_DATA_DIR` | Data folder | platform default | where `settings.json` and `mode.json` live |
| `ROCKET_SECRET_KEY` | Sign-in cookie secret | generated | signs the session cookie; set it for a server so sessions survive restarts |

Three keys have no Settings label and are set in the environment only:
`PROVIEW_SETTINGS_WINDOW_MS` is the window, in milliseconds, that one settings
read or write waits for the camera (default `20000`), `PROVIEW_SETTINGS_BACKUP_DIR`
is where the pre-write copies go (default `settings-backups` under the data folder),
and `PROVIEW_DVRIP_WINDOW_MS` is the window, in milliseconds, that one camera
operation waits — a capability read, a tour listing or change, a PTZ move, a
white-light or record-mode read, a snapshot, or the logout after it (default `20000`).

Nothing here is a vendor secret. The extracted application credentials from the
APK are deliberately **not** in this repository; `docs/protocol.md` records
where they come from and what they are used for, not their values.

## Layout

```
crates/core          config, providers (cloud/local/fixture), RTSP, DVRIP, ONVIF,
                     media (RTP, HEVC, fMP4), lens
crates/server        Rocket app: JSON API, static files, settings store, sessions
src-tauri            desktop shell: embedded server, keychain, restart command
web                  UI (no build step): app.js, index.html, style.css, tokens.css,
                     fonts/ (IBM Plex, SIL OFL 1.1)
fixtures             recorded device metadata for local runs
tools                measurement and contrast scripts
tests/web            UI tests in headless Chrome over the DevTools protocol (node --test)
docs                 protocol record, contracts, deploy and install guides
```

## Download and signing status

Installers and server binaries come from one release per tag (v0.3.5 is the
current one), with `SHA256SUMS` covering every file. None of them are
code-signed — there is no Apple Developer certificate and no Windows signing
certificate behind this project.

| Platform | Signature | First-launch handling | Upgrade path |
|---|---|---|---|
| Windows (msi/nsis) | unsigned (2026-10-27) | SmartScreen → More info → Run anyway; `Unblock-File` first | SignPath Foundation, or Certum Open Source Code Signing |
| macOS (dmg) | unsigned, ad-hoc `codesign -s -`, no notarization (2026-10-27) | right-click → Open, or `xattr -dr com.apple.quarantine` | Apple Developer ID + notarization |
| Linux (AppImage/deb/rpm) | not required | `chmod +x` for AppImage | — |

Details per platform: `docs/install.md`. What changed between releases:
`CHANGELOG.md`.

## Licence

MIT, see `LICENSE`.

This project ships no vendor code, no APK, no vendor `.so` and no extracted
keys. It talks to a device and a cloud account you own.
