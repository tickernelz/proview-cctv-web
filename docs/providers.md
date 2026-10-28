# Providers: cloud and local

The application ships two ways to reach a camera behind one port. The UI never
branches on the vendor protocol; it asks the provider for devices, streams,
recordings and settings.

| Provider | Default | Transport | What it can do today |
|---|---|---|---|
| `cloud` | **yes** | ProView account → vendor cloud (CSS/PSS, P2P) | account login and device list work through the signed v3 envelope in `crates/core/src/cloud/envelope.rs` (`POST /api/login`, `GET /api/devices`), with no vendor binary at run time. The v2 portal login is implemented and answers; the white-label account rejects it. One vendor cloud call works without a session: device caps by serial (`GET /api/cloud/device-caps`), see `cloud-app-surface.md`. Streaming, playback and PTZ over the cloud are still unimplemented |
| `local` | opt-in | LAN: DVRIP 34567, RTSP 554, HTTP 80 | RTSP `DESCRIBE` with Digest works against a real IPC; DVRIP login returns `ret 203` |

## Selection rules

- `PROVIEW_MODE=cloud|local`, default `cloud`. The choice is per profile in the
  UI; the environment variable is the bootstrap default.
- `local` requires `PROVIEW_DEVICE_HOST` (DVRIP) and/or `PROVIEW_DEVICE_RTSP`
  plus device credentials. With no host configured, the UI must say so and fall
  back to `cloud` — never a blank screen.
- The active provider is always visible in the interface. A stream's origin is a
  fact about the picture, not an implementation detail.

## Mode store

`crates/server/src/config.rs` holds the choice, so a mode survives a restart
where the platform has somewhere to keep it:

| Platform | Where the choice lives | Lifetime |
|---|---|---|
| Desktop | `<app_data_dir>/mode.json`, written atomically (staged file + rename) | until the operator changes it |
| Server | in memory, keyed by the `pvw_session` cookie | the session; logout drops it |

`ModeStore::from_env` reads `PROVIEW_MODE` for the bootstrap default and
`PROVIEW_DATA_DIR` (or the Tauri `app_data_dir` passed through `ServeOptions`)
for the persistence directory. An unreadable or malformed file falls back to the
environment default, which falls back to `cloud`.

## The API the UI reads

`GET /api/config/mode` → the requested mode, the provider that will answer, and
a notice when the two differ:

```json
{ "mode": "local", "source": "cloud",
  "notice": { "from": "local", "to": "cloud",
              "message": "this network is selected but no camera address is set yet; add one under Settings, Configuration, Local network, then restart the app." } }
```

`POST /api/config/mode {"mode":"local"}` needs a live session and returns the
same report. `GET /api/config/mode` works before sign-in and reports the
bootstrap default. Every stream ticket carries `source`
(`cloud`/`local`/`fixture`), so the player marks the origin of the picture.

## Fallback

`crates/core/src/provider/router.rs` decides once per request and then keeps the
decision:

1. `mode=cloud` → the configured provider (`CloudProvider`, or the fixture in
   tests and demos).
2. `mode=local` with a host → `LocalProvider` first, `CloudProvider` behind it.
3. `mode=local` without a host → `CloudProvider` immediately, with the notice
   above. The screen fills with cloud devices instead of staying empty.
4. `LocalProvider` fails with a transport error (refused, unreachable, timeout)
   → the router retries the same call on the cloud provider, records one notice,
   and stays on cloud for later calls so the UI does not stall on every retry.
5. `LocalProvider` rejects the credentials → `BadCredentials`, no fallback: a
   wrong device password must not silently authenticate a cloud account.

## Verified against real hardware (2026-10-26)

Camera: single IP camera (IPC) at `192.168.0.100`, not a DVR/NVR.

| Probe | Result |
|---|---|
| TCP 34567 (DVRIP) | open |
| TCP 554 (RTSP) | open, banner `H264DVR 1.0` (the OEM RTSP server name, also used on IPCs) |
| `DESCRIBE /` without credentials | `401`, Digest realm `b891e7cc20b4a341` |
| `DESCRIBE /` with Digest credentials | `200`, SDP: `m=video 0 RTP/AVP 98`, `a=rtpmap:98 H265/90000`, `a=framerate:25`, `m=audio 0 RTP/AVP 8`, `a=rtpmap:8 PCMA/8000` |
| DVRIP login (`LoginType: DVRIP-Web`) | `Ret: 100`, `SessionID 0x00000017`, `DeviceType "IPC "`, `ChannelNum 1` with the vendor "sofia" 8-character hash. A login sent before the random-then-hash exchange is answered `Ret: 203`, which is where the early `ret 203` report came from |

This camera offers **H.265 only** on both `stream=0` and `stream=1`, with G.711
A-law audio. Consequences for the live path:

1. H.265 passthrough only where the webview can decode HEVC (WebView2 with the
   OS codec, WKWebView on Apple silicon). Detected at runtime.
2. Otherwise one upstream transcode H.265→H.264 per (device, channel, stream),
   fanned out through the stream hub — never one transcode per viewer.
3. CPU and RSS for both paths must be measured and recorded in
   `docs/performance.md` before either is called done.

## Credential handling

- Credentials never appear in a URL, a log line or a capture.
- Desktop: the OS keychain (see below) holds the camera login for interactive
  sign-in. Server: nothing is stored unless the operator saves it in Settings,
  and then the camera password is written to `settings.json` in the data
  directory — file permissions are the whole protection there, so point
  `PROVIEW_DATA_DIR` at a directory only the service user can read.
- A secret is written once and never read back: `GET /api/settings` reports
  `set: true` and `value: null` for it.
- `src-tauri/src/keychain.rs` stores one JSON blob per profile under service
  `proview-cctv` through `keyring` (macOS Keychain, Windows Credential Manager,
  Secret Service on Linux). Profiles default to `default` and are named by
  `PROVIEW_PROFILE`.
  `proview-desktop --store-credentials` reads `{"user":…,"password":…}` from
  stdin, `--forget-credentials` deletes the entry, and startup loads the entry
  before falling back to `PROVIEW_DEVICE_USER`/`PROVIEW_DEVICE_PASS`. Passwords
  never appear in argv.
- RTSP `Content-Base` echoes the URL it was given; the client strips userinfo
  before logging and sends credentials only in `Authorization`.
- The `AdminToken` a password login returns is kept per serial in
  `device-tokens.json` (AES-256-GCM under `device-token.key`, both in the data
  directory), so the next open can log in with the token instead of the password.
  A camera that answers `Ret` 117 or 137 to that token login is marked refused for
  the life of the process: no token is offered to it again, the `AdminToken` its
  password login returns is not stored, and the open runs the password login on its
  first tunnel instead of paying a second one. Any other non-100 answer drops the
  token and tries the token path again on the next open, and a token answer of 100
  clears the mark.
