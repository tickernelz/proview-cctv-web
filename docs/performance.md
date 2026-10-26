# Performance

Rule for this repository: a number that was not measured is not a number. Every
change that touches the frame path records before/after here, produced by a
script under `tools/` so anyone can repeat it.

## Live path, local RTSP (measured 2026-10-26)

Camera: PROVIEW IPC at `192.168.0.100`, RTSP 554, main stream
2304×2592 H.265 (25 fps declared, 12 fps delivered), sub stream 640×720 H.265.
Server: release build on a Ryzen-class workstation under WSL2, single run per
row. Driver: headless Chromium over CDP, clicking the device row and polling
`video.readyState`/ `currentTime`, so the number includes RTSP setup, the
protocol exchange, the first keyframe, the transcode and the MSE append.

| Metric | Value |
|---|---|
| click → first decoded frame, cold source | 2.5 s / 5.3 s / 2.6 s (three runs) |
| click → first decoded frame, warm source (last fragment replayed) | 168 ms / 328 ms |
| RTSP open → first video NAL | 389–765 ms |
| access units pulled in 6 s (main stream) | 72 (matches `ffmpeg -i` control: 72) |
| media bytes pulled in 6 s (main) | 557–575 KB |
| sub stream 640×720, 4 s | 47 access units, 67 KB |
| WebSocket payload per 8 s while streaming | 7.5–10 MB (H.264 baseline out) |
| server RSS idle | 8.9–9.2 MiB |
| server RSS while one viewer streams | 16–38 MiB |
| server RSS after the viewer left, source warm | 20–33 MiB |

Reading the numbers:

1. Cold start is dominated by the camera's IDR interval (~2–3 s at 12 fps): the
   server transcodes from the first keyframe, and the device offers no way to
   request one over this RTSP session. Warm start replays the last complete
   fragment, which is why it drops to ~200 ms. The 60 s warm window after the
   last viewer leaves is what turns device switching into a warm start.
2. The stream is re-encoded to H.264 baseline because the IPC delivers H.265
   only; the sub stream is the cheap option at 640×720.
3. RSS while streaming is not flat: the broadcast ring holds up to 8 fragments
   and each fragment is 2–4 MB at this resolution. That is the first thing to
   attack before a 4-tile grid, together with a bitrate cap
   (`-maxrate`) so a fragment cannot grow past the ring budget.

## Recorded clip path (measured 2026-10-26, commit `a2a87ae`)

```sh
cargo build --release -p proview-server
./tools/measure-clip.sh target/release/proview-server
```

640×360 baseline clip, 72 frames, 47 KB source:

| Metric | Value |
|---|---|
| first byte (cold, includes muxing) | 1.6 ms |
| full body (cold) | 1.8 ms |
| first byte (warm, mean of 10) | 0.5 ms |
| full body (warm, mean of 10) | 0.7 ms |
| body size | 48 776 B |

## Targets and what is still open

| Metric | Target | State |
|---|---|---|
| click → first decoded frame, warm source | < 1.2 s | met (168–328 ms) |
| click → first decoded frame, cold source | < 1.2 s | not met (2.5–5.3 s), bounded by the camera IDR interval |
| CPU per viewer, live path | < 4 % of one core | not met by transcode (26.5–41.7 % of a core at 2304×2592, 3 viewers); met by passthrough (0.2 %) |
| RSS per active stream | < 40 MiB | not met by transcode (285–315 MiB server+ffmpeg at 2304×2592); met by passthrough (11–14 MiB) |
| 4-tile grid, mid-range laptop | no dropped frames, < 25 % of one core | not implemented; the grid is a single stage today |

## Design decisions behind the numbers

1. **One upstream per (device, channel, stream).** The hub broadcasts fMP4
   fragments to every viewer and keeps the init segment plus the last fragment,
   so a joining viewer starts on a complete keyframe instead of a partial GOP.
2. **Warm window instead of reconnect cost.** A source keeps running for 60 s
   after the last viewer leaves; the camera's IDR interval makes a cold start
   the dominant cost otherwise.
3. **Bounded buffers.** The hub ring is 8 fragments; the browser keeps a 12 s
   window and jumps to the live edge when it falls more than 2 s behind, so
   memory does not grow with uptime.
4. **Single transcode per stream.** H.265→H.264 happens once per
   (device, channel, stream) in `ffmpeg`, not per viewer; viewers share the
   output.

## Lens metadata endpoint (measured 2026-10-26)

`GET /api/lens` reads the frame size out of the SPS, so the whole call is RTSP
`DESCRIBE` work and it never touches the media path.

| Metric | Value |
|---|---|
| full call, main + sub, live camera (HTTP 200) | 457 ms |
| handshakes per call | one per requested quality, so two by default and one with `quality=main` |
| bytes added to a dual-lens view | 0 (the crop is client-side geometry, the stream is unchanged) |
| server CPU added by a dual-lens view | 0 (no second session, no second transcode) |

The endpoint is metadata, not a frame path: calling it per device selection does
not move the numbers in the tables above.

## Stream 2 live transports (measured 2026-10-26, `feat/stream2`)

Same camera as above (PROVIEW IPC, `192.168.0.100`, main stream 2304×2592
H.265, ~12 fps delivered), same workstation under WSL2, release build, Chrome
156 headless driven over CDP. One run per row. The box was also running four
other streams' dev servers, so the CPU column is an upper bound for an idle
box, not a floor. A passthrough row is only valid when the transcode source for
the same camera has already idled out (60 s of idle, noticed within 3 s); the run at 94 % in
this session was a transcode child still alive during a passthrough sample and
is not quoted below.

Reproduce:

```sh
cargo build --release -p proview-server
PROVIEW_DEVICE_RTSP=192.168.0.100:554 PROVIEW_DEVICE_USER=… PROVIEW_DEVICE_PASS=… \
  PROVIEW_MODE=local ROCKET_SECRET_KEY=$(head -c 32 /dev/urandom | base64) \
  ./target/release/proview-server --listen 127.0.0.1:8790
PROVIEW_MEASURE_EMAIL=… PROVIEW_MEASURE_PASSWORD=… \
  PROVIEW_BASE=http://127.0.0.1:8790/ PROVIEW_TRANSPORT=avc PROVIEW_VIEWERS=1 node tools/measure-live.mjs
sleep 70
PROVIEW_MEASURE_EMAIL=… PROVIEW_MEASURE_PASSWORD=… \
  PROVIEW_BASE=http://127.0.0.1:8790/ PROVIEW_TRANSPORT=hevc PROVIEW_VIEWERS=3 node tools/measure-live.mjs
```

The server decides the provider from `PROVIEW_MODE` (`local` needs no cloud
account and logs in against the camera's own RTSP credentials); the tool takes
the transport from `PROVIEW_TRANSPORT` because the same variable name would
collide with the provider switch.

| Metric | transcode (MSE, H.264 out) | passthrough (WebCodecs, H.265 out) |
|---|---|---|
| processes serving the stream | server + 1 `ffmpeg` | server only |
| click → first painted frame, cold source | 2.6 s, 5.0 s, 5.3 s (three runs) | first keyframe on the socket 0.56–0.78 s |
| click → first painted frame, warm source | 240 / 120 / 120 / 111 ms | not offered: a passthrough viewer always joins at the next IDR |
| CPU, 1 viewer | 63.5–85.4 % of one core | 0.1–0.2 % |
| CPU, 2 viewers | 83.3 % (41.7 % per viewer) | not measured |
| CPU, 3 viewers | 79.5–81.5 % (26.5–27.2 % per viewer) | 0.5 % (0.2 % per viewer) |
| RSS, server + children, 1 viewer | 283.1–314.7 MiB (server alone 23.9 MiB) | 11.4–33.9 MiB: the server is the only process, and the figure grows when a transcode stream ran earlier (allocator retention) |
| RSS, server + children, 3 viewers | 284.8–294.0 MiB | 13.6 MiB |
| wire bytes per 8 s, one viewer | 8.6 MB (baseline H.264) | 0.75–1.11 MB (H.265 untouched) |
| wire bytes per 8 s, three viewers | 25.8 MB (3 × 8.6) | 2.3–3.3 MB (3 × the one-viewer figure) |
| audio | PCMA/8000 counted (2122 packets in 8 s), not forwarded | same |

Reading the numbers:

1. One upstream per stream, not per viewer: the CPU column stays flat from one
   to three viewers and there is exactly one `ffmpeg` child, which is why the
   per-viewer cost falls from 81 % to 27 %. A grid of four tiles pays four
   streams, not four viewers per stream.
2. Passthrough removes that cost instead of sharing it: 0.2 % of one core and
   11 MiB for the same camera, because the H.265 bitstream is forwarded as
   access units instead of being decoded and re-encoded. It is also 8–11×
   cheaper on the wire (0.75–1.11 MB vs 8.6 MB per 8 s). The hub ring is sized
   per transport — 8 fragments for transcode (each starts on a keyframe) and 96
   access units for passthrough, about two GOPs at 12 fps — so a viewer that
   jitters by half a second does not have to wait for the next IDR.
3. Cold start for the transcode transport is the sum of RTSP setup, the
   camera's IDR interval and ffmpeg's first output: `frag_keyframe` holds a
   fragment until it can start on a keyframe, so the first painted frame
   costs 2.6–5.0 s. The passthrough column reports the first keyframe
   **on the socket** (0.56–0.78 s), which is the camera's own IDR cadence with
   a warm RTSP session — a different stopwatch, not a cheaper cold start.
   Warm starts are 111–240 ms because the hub replays the newest
   keyframe-start fragment (transcode); passthrough has no replay and always
   waits for the camera's next IDR.
4. The frame really painted on this machine: Chromium reported `readyState 4`,
   `videoWidth 2304`, `videoHeight 2592` and an advancing `currentTime`, and a
   screenshot of that moment measured `YAVG 171.8`, `YMIN 16`, `YMAX 235` — a
   real scene, not a black canvas. Passthrough cannot paint here: Chromium 156
   on Linux answers false to `VideoDecoder.isConfigSupported({codec:'hvc1…'})`
   and to `MediaSource.isTypeSupported('…hvc1…')`, so its pixels were proved by
   feeding the exact socket payload to `ffmpeg -f hevc -i pipe:0 -f null -`:
   3 keyframes + 85–92 deltas in 8 s, exit 0, empty stderr.

Defect found by measuring, fixed here: the MSE transport was receiving its init
segment wrapped in the passthrough config framing (one leading `0x01` byte), so
`MediaSource` rejected the segment and the element stayed at `readyState 0`
behind a blob URL. Before: zero frames, every run. After: 111–240 ms to the
first painted frame on a warm source. Framing is now chosen by transport —
init and fragments raw for MSE, `0x01`-prefixed hvcC plus `0x02`/`0x03` access
units for WebCodecs.
