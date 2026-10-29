# Performance

> Cloud login, 2026-10-27: the Rust envelope path needs 0.30-0.39 s for the
> product path (sign-in, then the device list), against 65 s for the qemu SDK
> harness. The per-step split, including the example's extra device-token and
> refresh calls at 0.63-0.94 s, is in section "Cloud account login".


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

## Settings API (measured 2026-10-27, release `v0.2.4`)

`GET /api/settings` builds its answer from the static field table and the
process environment; the file is read once at startup, so the call never touches
the frame path. Measured on the shipped
`proview-server-x86_64-unknown-linux-gnu` binary from `v0.2.4`, empty data
directory, 20 fields, loopback:

| Metric | Value |
|---|---|
| `GET /api/settings`, three runs | 0.49 ms / 0.50 ms / 0.44 ms |
| `POST /api/settings`, one key | one staged write plus rename of a file under 1 KB |

Reproduce:

```sh
for i in 1 2 3; do curl -s -o /dev/null -w '%{time_total}\n' http://127.0.0.1:8787/api/settings; done
```

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

## Cloud account login, Rust envelope (measured 2026-10-27)

Driver: `cargo build -p proview-core --example envelope_login` and then
`./target/debug/examples/envelope_login` on this workstation (WSL2, Ryzen
class). Debug build for the table below, plus two `--release` runs of the same
example for contrast: 0.67 s and 0.83 s end to end, i.e. inside the debug
range, because the cost is network bound. Three consecutive debug runs,
2026-10-27, one account, one device. Tokens are printed truncated; nothing in
the run depends on them being logged.

| Step | Run 1 | Run 2 | Run 3 |
|---|---|---|---|
| `loginExist` (email + password -> accessToken, refreshToken) | 0.33 s | 0.25 s | 0.24 s |
| platform exchange + sealed `/deviceToken/get` | 0.39 s | 0.21 s | 0.20 s |
| `msdlistCode` device list (1 device) | 0.06 s | 0.07 s | 0.06 s |
| `refreshToken` | 0.13 s | 0.13 s | 0.13 s |
| product path: `loginExist` then `msdlistCode` (what `CloudProvider` does) | 0.39 s | 0.32 s | 0.30 s |
| example end to end (all four steps above) | 0.94 s | 0.66 s | 0.64 s |
| `re/sdk_login.sh`, the same login + device list through qemu | 65 s (measured once, 2026-10-27 03:45Z, notes 12 section 10) | | |

The in-process product path is 0.30-0.39 s against 65 s for the qemu harness,
about 170x on the same box and the same account; the 65 s was recorded once by
that harness run, so it is a reference point, not a distribution. Reading the
split:

1. The protocol is cheap: each signed call costs 60-400 ms, which is round
trip plus AES/MD5 over a ~110-character string. Nothing in the envelope needs
more than a millisecond of CPU.
2. The harness spends its 65 s on the emulation, not on the protocol: a
chrooted aarch64 rootfs under qemu-user, an SDK init chain (`SetApplication`,
`Init`, `SysInitNet`, `XmCloudPlatformInit`, `LogInit`) and a second `run_mode`
for the device list. Any comparison that quotes 65 s is comparing process
start-up, not the login.
3. The device token step is the slowest of the three because it performs two
HTTP calls (platform exchange, then the sealed call) and, per run, an RSA-1024
PKCS1v1.5 encryption plus one AES block chain on the client.
4. `refreshToken` returning `same=false` in two of three runs and `same=true`
in one shows the endpoint really issues a new token; the `true` case is a
refresh inside the same second as the login, i.e. the server returns the token
it already minted.

Reproduce:

```
PROVIEW_EMAIL=... PROVIEW_PASSWORD=... \
PROVIEW_APP_UUID=... PROVIEW_APP_KEY=... PROVIEW_APP_SECRET=... \
cargo run -p proview-core --example envelope_login
```

A release build already carries the identity and needs neither of the three; the
example is a source build, so it takes them from the environment or from
`cloud-identity.env` at the workspace root.

## Cloud live viewer cost (counted 2026-10-28)

What one viewer open asks the vendor cloud for, counted against a stub cloud
that answers `msdlistCode` and nothing else:

| Path | Before | After |
|---|---|---|
| `GET /api/stream`, first open | 1 `msdlistCode` + a new client and token-store load | 1 `msdlistCode`, shared client |
| `GET /ws/live`, same viewer | 1 `msdlistCode` + a new client and token-store load | 0 |
| cold start (`open_media`) | 1 `msdlistCode` + a new client and token-store load | 0 |
| total per viewer open | 3 | 1 |
| N parallel cold opens, one session | N `msdlistCode` | 1 |
| N parallel opens for serials the list lacks | N `msdlistCode`, N x RTT | 1 |
| N parallel opens against a failing cloud | N `msdlistCode`, N x failure | 1 |

`crates/server/tests/live_scope.rs` measures the row: one ticket, then two
socket opens on the camera the account owns, and one `msdlistCode` request
arrives. The window is 30 s per vendor session, and the next call after it
closes asks the cloud again, a serial the window does not carry is asked again
rather than refused, and a fresh device-list fetch (`/api/devices`) replaces the
window; `crates/core/src/provider/cloud.rs` carries the twelve tests behind those
sentences (one session pays one list for every serial, a second session pays its
own, an entry older than the window is fetched again, a serial bound inside the
window is recognised, a refresh replaces the window, a serial a refresh drops
stops answering, three parallel cold opens on one session pay one list, four
parallel opens for serials the list lacks pay one, four parallel opens against a
failing cloud pay one and return in one failure's time, a refresh that arrives
while another fetch is running asks again, a failure is not remembered for the
next open, and the gate map does not grow with each session).
The fetch is single-flight per vendor session: a `tokio::sync` mutex per token
gates it and is dropped once no caller holds it, so the winner asks the cloud and
a lookup waiter adopts what it produced. Adoption is keyed on when the caller
arrived, not on the serial: a list or a failure that finished after the caller
started waiting is the answer, so a burst of opens pays one call whether it hits,
misses or fails, and a waiter never queues behind an attempt that already
finished. An explicit refresh — `GET /api/devices`, `devices()` — never adopts:
it waits for the gate and then asks, so a camera bound or a nickname changed
while another fetch was running is still in the list it returns. A sequential
open still refetches, which is what keeps the refetch-miss above a miss and not a
refusal; the cost of adoption is that a camera bound inside one round trip is not
seen by the opens already waiting, and the next open or `/api/devices` picks it
up.
No wall-clock claim here: the list is 0.06-0.07 s on this box (table above), so
the fix removes two of those round trips and three client builds per open.

## Live cloud acceptance (measured 2026-10-28)

`tools/acceptance/live_acceptance.py` is the only evidence this repository
accepts for "live cloud works"; `docs/live-acceptance.md` says what it refuses to
claim. These are the first three runs, all from the same WSL2 host whose LAN
cannot reach the camera (`nc -z 192.168.0.100 554` times out after 3.0 s), so
every byte in them had to cross the vendor cloud.

| Run | Code | Verdict | Where it stopped | Numbers |
|---|---|---|---|---|
| `4d9ff0773885-20261028T014711Z` | `4d9ff07`, fixture self-check | pass | - | 31 messages, 279 010 B, 30.0 s, longest gap 1.0 s; `ffprobe` decodes 700 frames at 640×360 H.264 |
| `4d9ff0773885-20261028T015118Z` | `4d9ff07`, main | fail | `stream` | relay `103.85.122.25` answers `ret 117`, ticket 502 in 0.398 s |
| `56771819ad88-20261028T015228Z` | `5677181`, `work/stream-live` | fail | `stream` | 0 keyframes, 0 deltas, 0 dropped, 0 elementary bytes from the same relay |

What the stages that do pass cost, on loopback plus vendor time:

| Stage | Run `…T015118Z` | Run `…T015228Z` |
|---|---|---|
| camera unreachable on the LAN | 3.031 s | 3.005 s |
| `cargo build --release -p proview-server` | 1.329 s (warm) | 93.950 s (branch switch) |
| `POST /api/login`, signed v3 envelope | 0.208 s | 0.310 s |
| `GET /api/devices`, one device, serial `7f92970ddf5282743hrc` | 0.276 s | 0.299 s |
| `GET /api/stream` | 0.398 s, HTTP 502 | 3.468 s, HTTP 502 |

The first build that bakes the app identity (`PROVIEW_APP_*` from
`secrets/identity.env`) took 89.888 s in run `…T014902Z`; afterwards the build
stage is an up-to-date cargo check.

Reading the numbers:

1. Sign-in and the device list are not the problem. They reproduce the
   2026-10-27 envelope row (0.30-0.39 s) at 0.21-0.31 s here, and the device
   list carries the camera serial in 0.28-0.30 s.
2. The relay accepts the claim and then delivers nothing. Main never sends the
   start (`ret 117`, "cipher and start pending"); `work/stream-live` sends the
   claim before login and still counts zero media in six seconds. The missing
   piece is the start payload or the cipher, not the connection.
3. The fixture self-check is the harness proving itself, not evidence about the
   camera: the same counter runs over a recorded clip, and the dump decodes to
   700 frames in 30 s (23.3 fps inside 31 fMP4 fragments). A zero in the cloud
   rows is therefore a relay fact, not dead code.

Reproduce:

```sh
cd ~/Projects/proview-cctv-web-wt/acceptance
python3 tools/acceptance/live_acceptance.py --sn 7f92970ddf5282743hrc
```

No pass can be recorded until the `stream` stage stops failing. Each run keeps
`report.md`, `report.json`, the server log and the payload dump under
`~/re/proview-cctv/acceptance/`.

### Round 2: controls after review (measured 2026-10-28)

The first review asked for negative controls, so these ran against the harness
itself: same host, same LAN guard, no camera involved.

| Control | Setup | Result |
|---|---|---|
| camera reachable on the LAN with `--allow-lan` | a local listener on `127.0.0.1:<port>` passed as `--lan-target` | verdict `lan-reachable`, exit 3, stage `stream` also recorded; report `a15a5d88ebb8-20261028T021302Z` |
| stale binary under `--no-build` | the release binary copied aside with mtime set to 2026-01-01 | exit 1 at `build`: "would label /tmp/zha4085-old-server (built 2026-01-01T00:00:00Z) as commit 2026-10-28T01:55:01Z" |
| fixture self-check after the fixes | `--mode fixture --duration 20 --sample 5` | verdict `pass-fixture`, exit 0, 22 messages / 195 842 B / 20.204 s, longest gap 1.001 s |
| cloud path after the fixes | `--sn 7f92970ddf5282743hrc --duration 300` | exit 1 at `stream`, 502 `ret 117` in 0.398 s |

Framed dump, server-format feed (a probe, not a camera run): 100 HEVC access
units from libx265 at 640×360 (10 of them carrying VPS/SPS/PPS, as
`ensure_parameter_sets` does), each AU sent the way `hevc::avcc` sends it, NALs
with a four-byte big-endian length prefix, `hello` codec `hvc1.1.6.L90.90`.

The dump is 70 906 bytes and byte-identical to an Annex-B rebuild of the same
NALs, because every length prefix becomes a `00 00 00 01` start code. `ffprobe
-f hevc` reads 100 frames at 640×360 with empty stderr. Left as the server sends
them, the same bytes are unreadable: `ffprobe -f hevc` answers `hevc,0,0,N/A`
with "missing picture in access unit with size 70906", so the conversion is what
makes the artifact verify, not decoration.

## Web bundle and e2e suite (measured 2026-10-28, ZHA-4181)

Bundle: `npm run build` in `apps/web`, then `gzip -9c <file> | wc -c` per file.
Baseline is origin/main 6405caf; v2 is the redesign commit 89976a1 (lucide
sprite, tokens v2, no injected stylesheets).

| File | main 6405caf | v2 89976a1 | Delta | Budget |
|---|---:|---:|---:|---:|
| JS (one hashed chunk) | 40,396 B | 39,133 B | -1,263 B | +80 KB |
| CSS (one hashed file) | 5,450 B | 6,792 B | +1,342 B | +20 KB |
| index.html (inline sprite) | 6,336 B | 7,104 B | +768 B | - |

JS shrank because three scripts no longer carry their CSS as strings; the CSS
grew by the same rules moved into `style.css` plus the accent states.

e2e harness, same 84 tests (36 functional + 48 screenshots, Chromium + WebKit),
2 workers, retry 1, `mcr.microsoft.com/playwright:v1.64.1-noble`, fixture
servers restarted before each cold run, three rounds:

| Harness | Cold | Warm | Median |
|---|---|---|---|
| tester-army/e2e 0.21.3 + @e2e-dev/web 0.15.3, `--strict-cache` | 76 / 64 / 54 s | 70 / 72 / 76 s | 71 s |
| @playwright/test 1.64.1 | 45 / 43 / 50 s | 44 / 42 / 46 s | 44.5 s |

CI, run 40726996679 at 1edca06: job `e2e` 219 s (container init 38 s, cached
`cargo build` 64 s, tests 66 s). It runs beside `check` (224 s), so the
pipeline wall is 227 s; main 7510ccd took 385 s, held by a 357 s macOS desktop
job. The slowest single test is 4.9 s (WebKit, alarms). With the responsive and touch
specs the suite is 218 tests, 1.6 min locally.

### Live toolbar in Vue (measured 2026-10-29, ZHA-4181)

The caption, the control row and the lens and quality menus are one Vue
component (`apps/web/src/live/LiveBar.vue`) over a reactive store
(`src/live/store.ts`); the menus are Reka UI 2.11.0 `DropdownMenu`, vendored as
shadcn-vue style wrappers in `src/components/ui/dropdown-menu/`. Same method as
above, baseline origin/main 98db3a5 (Vue and Tailwind compiled, nothing mounted):

| File | main 98db3a5 | Live in Vue | Delta | Budget |
|---|---:|---:|---:|---:|
| JS (one hashed chunk) | 39,469 B | 106,865 B | +67,396 B | +80 KB |
| CSS (one hashed file) | 7,004 B | 7,115 B | +111 B | +20 KB |

The JS delta is the first mounted Vue app: the Vue runtime, Reka UI with
`@vueuse/core` and floating-ui for menu placement. Later screens reuse all of
it, so they add only their own components. Icons stay in the inline sprite
(`Icon.vue` renders `<use href="#i-name">`), so no icon package ships.
e2e at this change: 139 passed, 1 skipped, 1.3 min locally; node web tests 46/46.

### Cloud recordings in Vue (measured 2026-10-29, ZHA-4181)

`src/live/RecordingsPanel.vue` replaces `legacy/recordings.js`. Same method,
baseline origin/main 05fe8a3:

| File | main 05fe8a3 | Recordings in Vue | Delta |
|---|---:|---:|---:|
| JS | 106,865 B | 107,380 B | +515 B |
| CSS | 7,115 B | 7,131 B | +16 B |

e2e at this change: 159 passed, 1 skipped (5 recordings specs and 8 screenshot
baselines added), 1.4 min locally; node web tests 46/46.

### Alarm messages in Vue (measured 2026-10-29, ZHA-4181)

`src/live/AlarmCenter.vue` replaces `legacy/alarms.js`. Same method, baseline
origin/main e3db222:

| File | main e3db222 | Alarms in Vue | Delta |
|---|---:|---:|---:|
| JS | 107,402 B | 107,906 B | +504 B |
| CSS | 7,129 B | 7,173 B | +44 B |

The sheet reads the camera list the app already loaded, so opening it no longer
sends its own `/api/devices` (one vendor status query per camera in cloud
mode).

Vendor calls per visit, counted by routing `/api/cloud/alarms`,
`/api/cloud/alarms/picture` and `/api/devices` in Chromium against the
fixture server (`pollSeconds` 20, one camera, three alarms with pictures):

| Window | main e3db222 (alarms / pictures / devices) | first Vue port 40794ae | this change |
|---|---|---|---|
| first 5 s after sign-in | 0 / 0 / 2 | 1 / 1 / 1 | 1 / 1 / 1 |
| then 3 × Settings and back to Live | 0 / 0 / 3 | 3 / 3 / 0 | 0 / 0 / 0 |
| total 25 s after sign-in | 0 / 0 / 5 | 4 / 4 / 1 | 2 / 1 / 1 |

The first alarm check now runs at sign-in instead of 20 s later, so the bell
shows its count at once; that costs one alarm-centre query and one picture
probe (a signed object request and one thumbnail GET, about 66 KB) on a visit
shorter than `pollSeconds`. The steady rate stays one check per `pollSeconds`.
Settings no longer counts as signing out: the last answer, the seen ids and
the picture probe result are kept, and coming back checks only when the last
check is older than `pollSeconds` (otherwise the remaining time is scheduled).
Main stopped its timer on Settings and restarted the full 20 s on return, so a
user moving between Live and Settings saw no check at all there.

e2e at this change: 183 passed, 1 skipped, 1.6 min locally; node web tests
46/46.

### Devices sheet in Vue (measured 2026-10-29, ZHA-4181)

`src/live/DevicesSheet.vue` replaces `legacy/device-admin.js`, with Reka UI
Tabs vendored as `src/components/ui/tabs/`. Same method, baseline origin/main
8960187:

| File | main 8960187 | Devices in Vue | Delta |
|---|---:|---:|---:|
| JS | 107,987 B | 110,859 B | +2,872 B |
| CSS | 7,173 B | 7,494 B | +321 B |

Most of the JS is Reka's Tabs (roving focus and the tab/panel wiring). Running
total against the pre-Vue baseline stays inside the +80 KB JS and +20 KB CSS
budgets. e2e at this change: 197 passed, 1 skipped, 1.9 min locally; node web
tests 46/46.
