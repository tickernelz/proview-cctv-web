# proview-cctv-web

A small Rocket server that puts PROVIEW CCTV (package `com.proview.cctv`, a
Xiongmai / XMEye white-label) cameras on a desktop browser instead of a phone.

Status: **protocol reconnaissance, skeleton server**. The web layer, session
handling and provider boundary are real; the vendor cloud calls and the live
video transport are not finished. See `docs/protocol.md` for exactly what is
proven and what is not.

## What works today

| Piece | State |
|---|---|
| Rocket server, static UI, JSON API | works |
| Server-side session (private cookie → in-memory vendor session) | works |
| Fixture provider (device list, stream ticket) | works, default |
| DVRIP framing, login handshake, session | works, tested against a mock device |
| Annex-B NAL splitter, H.264/H.265 detection | works, tested |
| Media packaging to fMP4 (playable) | works: Annex-B clip → init+moof+mdat, verified with ffprobe and in-browser playback |
| H.264 SPS parse (dimensions, profile) | works, tested |
| Vendor cloud login | partially mapped; the portal login encoder is recovered but the PROVIEW account lives on the white-label cloud, not the global portal |
| Device list from the vendor cloud | not implemented |
| Live video to the browser | works: RTSP pull + H.265 depacketize + one transcode per stream + fMP4 over WebSocket into MSE; verified against a real IPC |

The server never sends a vendor token to the browser: the browser holds only an
opaque `pvw_session` cookie, and every vendor call happens server side.

## Run

```sh
cargo run --release
# http://127.0.0.1:8000
```

Fixture mode is the default and needs no account:

```
email:    fixture@example.invalid
password: fixture-password
```

To point it at the real cloud:

```sh
export PROVIEW_PROVIDER=proview
export PROVIEW_CLOUD_BASE=https://pub-token.xmeye.net
cargo run --release
```

## Live view from a LAN camera

The local provider needs no cloud account: point the server at the camera's
RTSP endpoint and it pulls, depacketizes and transcodes the stream once per
channel, then fans the fragments out over a WebSocket.

```sh
export PROVIEW_DEVICE_RTSP=192.168.0.100:554
export PROVIEW_DEVICE_USER=admin
export PROVIEW_DEVICE_PASS='the device password'
cargo run --release -p proview-server
```

`ffmpeg` must be on `PATH` for HEVC cameras (or set `PROVIEW_FFMPEG` to the
binary). The camera in hand delivers H.265 only, so the server transcodes to
H.264 baseline once per (device, channel, stream) and every viewer shares the
result; browsers do not have to decode HEVC. Measured latency and memory are in
`docs/performance.md`, the protocol in `docs/protocol.md` section 8.

The stream source stays warm for 60 s after the last viewer leaves, so
re-opening a camera is a warm start (~200 ms) instead of a cold one (~2.5-5 s,
bounded by the camera's keyframe interval).

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `PROVIEW_PROVIDER` | `fixture` | `fixture` or `proview` |
| `PROVIEW_CLOUD_BASE` | `https://pub-token.xmeye.net` | vendor cloud base |
| `PROVIEW_DEVICE_HOST` | unset | `host:port` of the camera for the DVRIP probe |
| `PROVIEW_DEVICE_RTSP` | unset | `host:port` of the camera RTSP endpoint; when set, the UI streams from the LAN instead of the fixture |
| `PROVIEW_FFMPEG` | `ffmpeg` | ffmpeg binary used for the HEVC to H.264 bridge |
| `PROVIEW_DEVICE_USER` | `admin` | device-side account |
| `PROVIEW_DEVICE_PASS` | empty | device-side password |
| `PROVIEW_FIXTURE_DIR` | `fixtures` | fixture data directory |
| `PROVIEW_SESSION_TTL` | `3600` | server session lifetime, seconds |

Nothing here is a vendor secret. The extracted application credentials from the
APK are deliberately **not** in this repository; `docs/protocol.md` records
where they come from and what they are used for, not their values.

## Layout

```
src/config.rs        env configuration
src/session.rs       opaque session id → vendor session, in memory
src/provider/        the seam: fixture | proview
src/routes.rs        JSON API + static files
static/              UI (no build step)
fixtures/            recorded device metadata for local runs
docs/protocol.md     the reverse-engineering record
```

## Download and signing status

Installers and server binaries come from one release per tag, with
`SHA256SUMS`. None of them are code-signed — there is no Apple Developer
certificate and no Windows signing certificate behind this project.

| Platform | Signature | First-launch handling | Upgrade path |
|---|---|---|---|
| Windows (msi/nsis) | unsigned (2026-10-26) | SmartScreen → More info → Run anyway; `Unblock-File` first | SignPath Foundation, or Certum Open Source Code Signing |
| macOS (dmg) | unsigned, ad-hoc `codesign -s -`, no notarization (2026-10-26) | right-click → Open, or `xattr -dr com.apple.quarantine` | Apple Developer ID + notarization |
| Linux (AppImage/deb/rpm) | not required | `chmod +x` for AppImage | — |

Details per platform: `docs/install.md`.

## Licence

MIT, see `LICENSE`.

This project ships no vendor code, no APK, no vendor `.so` and no extracted
keys. It talks to a device and a cloud account you own.
