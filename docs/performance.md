# Performance

Rule for this repository: a number that was not measured is not a number. Every
change that touches the frame path records before/after here, produced by a
script under `tools/` so anyone can repeat it.

## What is measurable today

The live transport (device → server → browser) does not exist yet, so the only
end-to-end path is a recorded Annex-B clip converted to fragmented MP4 on
request. That path exercises NAL splitting, SPS parsing, the moov/moof/mdat
writer and Rocket's response, which is exactly the code the live path will
reuse.

```sh
cargo build --release -p proview-server
./tools/measure-clip.sh target/release/proview-server
```

Recorded on a Ryzen-class workstation (WSL2), single run, 640×360 baseline clip,
72 frames, 47 KB source:

| Metric | Value |
|---|---|
| first byte (cold, includes muxing) | see latest run below |
| full body (cold) | |
| full body (warm, mean of 10) | |
| server RSS after run | |

## Targets for the live path (not yet met)

These are commitments, not measurements:

| Metric | Target |
|---|---|
| click → first decoded frame | < 1.2 s on LAN |
| CPU per viewer, 720p H.264 | < 4 % of one core |
| RSS per active stream | < 40 MiB |
| 4-tile grid, mid-range laptop | no dropped frames, < 25 % of one core |

## Design decisions behind the numbers

1. **One upstream per (device, channel, stream).** A hub broadcasts fragments to
   all viewers; a second browser tab must not open a second vendor session. A
   joining viewer starts at the next IDR so it never decodes a partial GOP.
2. **Frame path stays allocation-light.** `bytes::Bytes`/Arc sharing, a reusable
   output buffer per viewer, slow clients dropped to the next keyframe instead
   of growing an unbounded queue.
3. **Allocator and runtime.** mimalloc (or jemalloc) as the global allocator,
   tokio multi-thread, one task per upstream plus one writer task per viewer,
   `TCP_NODELAY` on the WebSocket.
4. **Transport.** Binary WebSocket carrying fMP4 for live; HLS/FLV only for the
   vendor's own cloud restream.
5. **Frontend.** `VideoDecoder` (WebCodecs) first, MSE second, canvas paint via
   `requestAnimationFrame`; off-screen tiles paused with `IntersectionObserver`
   and the decoded-tile budget set from measurement.

## How each future number gets recorded

`tools/measure-clip.sh` is the template: start the binary on a fixed port, log
in, hit the endpoint N times, report time-to-first-byte, total time, byte count,
RSS and CPU seconds from `/proc`. A live-path script (`tools/measure-live.sh`)
will add: click→frame using the browser's `requestVideoFrameCallback`, and CPU
per viewer derived from `process_cpu_seconds_total` deltas.
