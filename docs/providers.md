# Providers: cloud and local

The application ships two ways to reach a camera behind one port. The UI never
branches on the vendor protocol; it asks the provider for devices, streams,
recordings and settings.

| Provider | Default | Transport | What it can do today |
|---|---|---|---|
| `cloud` | **yes** | ProView account → vendor cloud (CSS/PSS, P2P) | login envelope unsolved, so nothing is exposed yet |
| `local` | opt-in | LAN: DVRIP 34567, RTSP 554, HTTP 80 | RTSP `DESCRIBE` with Digest works against a real IPC; DVRIP login returns `ret 203` |

## Selection rules

- `PROVIEW_MODE=cloud|local`, default `cloud`. The choice is per profile in the
  UI; the environment variable is the bootstrap default.
- `local` requires `PROVIEW_DEVICE_HOST` (DVRIP) and/or `PROVIEW_DEVICE_RTSP`
  plus device credentials. With no host configured, the UI must say so and fall
  back to `cloud` — never a blank screen.
- The active provider is always visible in the interface. A stream's origin is a
  fact about the picture, not an implementation detail.

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
- RTSP `Content-Base` echoes the URL it was given; the client strips userinfo
  before logging and sends credentials only in `Authorization`.
