# Providers: cloud and local

The application ships two ways to reach a camera behind one port. The UI never
branches on the vendor protocol; it asks the provider for devices, streams,
recordings and settings.

| Provider | Default | Transport | What it can do today |
|---|---|---|---|
| `cloud` | **yes** | ProView account → vendor cloud (CSS/PSS, P2P) | portal account login is unsolved, and the app's own login is behind a pinned host. One vendor cloud call works without a session: device caps by serial (`GET /api/cloud/device-caps`), see `cloud-app-surface.md` |
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
| DVRIP login (`LoginType: DVRIP-Web`) | `ret 203` — credentials rejected; hash scheme for this firmware unsolved |

This camera offers **H.265 only** on both `stream=0` and `stream=1`, with G.711
A-law audio. Consequences for the live path:

1. H.265 passthrough only where the webview can decode HEVC (WebView2 with the
   OS codec, WKWebView on Apple silicon). Detected at runtime.
2. Otherwise one upstream transcode H.265→H.264 per (device, channel, stream),
   fanned out through the stream hub — never one transcode per viewer.
3. CPU and RSS for both paths must be measured and recorded in
   `docs/performance.md` before either is called done.

## Credential handling

- Credentials live in the OS keychain (desktop) or the operator's secret store
  (server). They are never written into a URL, a log line, or a capture.
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
