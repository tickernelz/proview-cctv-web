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
| CPU per viewer, 720p H.264 | < 4 % of one core | not measured yet for the live path (transcode cost unmeasured) |
| RSS per active stream | < 40 MiB | borderline (16–38 MiB at 2304×2592) |
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
