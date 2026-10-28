# Live acceptance

One command decides whether the cloud path really carried live video, and it
writes the numbers down.

```sh
tools/acceptance/live_acceptance.py
```

The harness is the only accepted evidence for a "live cloud works" claim. A
sentence in a chat, a screenshot of a playing tile, and a passing unit test do
not substitute for it, because none of them prove the relay carried frames for
five continuous minutes from a host that cannot reach the camera on the LAN.

## What a pass means

Exit code 0 means every stage below passed on the branch under test:

| Stage | Requirement |
|---|---|
| `lan_guard` | `nc -z 192.168.0.100 554` fails, so the camera is not reachable on the LAN and every byte must come through the vendor cloud |
| `relay_lock` | the run holds `~/re/proview-cctv/relay.lock`, so one live session at a time, as the vendor relay requires |
| `identity` | the three app-identity keys come from `~/re/proview-cctv/secrets/identity.env`; without them a build signs nothing and answers `MISSING_CLOUD_IDENTITY` at login |
| `build` | `cargo build --release -p proview-server` through `~/re/proview-cctv/build-slot.sh`, never a bare cargo invocation, with the identity exported as `PROVIEW_APP_*` and `PROVIEW_REQUIRE_CLOUD_IDENTITY=1` so a build that cannot bake it fails loudly |
| `credentials` | sign-in material from `~/re/proview-cctv/secrets/proview.env` |
| `serve` | the release binary answers `/api/health` on a random loopback port |
| `login` | `POST /api/login` returns 200 with `ok: true` |
| `mode` | `/api/config/mode` resolves to `cloud`, not a LAN or fixture provider |
| `devices` | `GET /api/devices` returns 200 and lists the camera; with `--sn` the body must carry that serial |
| `stream` | `GET /api/stream` returns 200 with a ticket |
| `websocket` | `/ws/live` upgrades with 101 |
| `live` | frames keep arriving for the whole `--duration` (default 300 s), no gap past 5 s |
| `keyframes` | at least one keyframe counted on the socket, or by the server at `/api/live/stats` |

Any failure exits non-zero and names the stage that broke, which is the point:
the harness is worth more when it fails.

| Exit code | Meaning |
|---|---|
| 0 | every stage passed |
| 1 | a stage failed; `report.md` names it |
| 2 | preflight: missing binary, missing build slot, missing credentials, missing app identity |
| 3 | the camera answered on the LAN, so the run cannot prove the cloud path |
| 4 | the relay lock stayed held by another session |

## What a pass does not mean

- It is not a parity claim. Parity is a row-by-row comparison against the
  vendor application, tracked in `docs/parity.md`; this harness only proves the
  relay carried video with numbers attached.
- It says nothing about the camera field of view, the app UI, the recordings,
  or the alarms.
- A pass counted only in cloud mode with the LAN guard reporting the camera
  unreachable. `--mode fixture` runs the same plumbing against a recorded clip
  on purpose, and never counts as cloud evidence.

## Running it

Two private files feed the harness: `secrets/proview.env` (`PROVIEW_EMAIL`,
`PROVIEW_PASSWORD`) for sign-in, and `secrets/identity.env`
(`PROVIEW_APP_UUID`, `PROVIEW_APP_KEY`, `PROVIEW_APP_SECRET`) which the build
bakes into the binary. No value from either file is ever written to a report;
the report records the file path and which keys were present.

Cloud run from a host off the camera LAN:

```sh
cd ~/Projects/proview-cctv-web-wt/acceptance
python3 tools/acceptance/live_acceptance.py --sn <camera-serial>
```

Useful flags:

| Flag | Default | Purpose |
|---|---|---|
| `--duration` | 300 | seconds of continuous streaming required |
| `--sample` | 10 | seconds between sample rows |
| `--sn` | none | serial that `GET /api/devices` must carry |
| `--device` | first device | override the device id sent to `/api/stream` |
| `--channel` | 1 | camera channel |
| `--quality` | main | `main` or `sub` |
| `--codec` | none | `hevc` asks for passthrough, anything else transcodes |
| `--lan-target` | `192.168.0.100:554` | address that must stay unreachable |
| `--allow-lan` | off | keep going when the camera answers on the LAN; the report records it, and the result never counts as cloud evidence |
| `--no-build` | off | reuse `target/release/proview-server` |
| `--bin` | `target/release/proview-server` | binary to run |
| `--out` | `~/re/proview-cctv/acceptance` | report root |
| `--lock` | `~/re/proview-cctv/relay.lock` | relay mutex |
| `--lock-wait` | 60 | seconds to wait for the lock before failing |
| `--identity-file` | `~/re/proview-cctv/secrets/identity.env` | `PROVIEW_APP_UUID`/`KEY`/`SECRET` handed to the build |
| `--env-file` | `~/re/proview-cctv/secrets/proview.env` | `PROVIEW_EMAIL`/`PASSWORD` used for sign-in |
| `--dump-bytes` | 4 MiB | cap on the elementary stream kept for ffprobe |
| `--keep-server` | off | leave the server running after the run |

### Plumbing self-check

The frame counter must be provably alive even while the cloud path is broken,
otherwise a zero cannot be told apart from dead code. `--mode fixture` runs the
same login, devices, ticket, websocket and counting code against a recorded
clip. It skips the LAN guard and is labelled `fixture` in the report.

```sh
mkdir -p /tmp/proview-acceptance-fixture
cp fixtures/account.json fixtures/devices.json /tmp/proview-acceptance-fixture/
ffmpeg -f lavfi -i testsrc=size=640x360:rate=25 -t 10 \
  -c:v libx264 -x264-params keyint=25 -f h264 \
  /tmp/proview-acceptance-fixture/fixture-dev-0001_ch1_main.h264
python3 tools/acceptance/live_acceptance.py --mode fixture \
  --fixture-dir /tmp/proview-acceptance-fixture --duration 30 --sample 5
```

A self-check that reports zero frames is a harness bug, never a camera bug.

## Reading the report

Every run writes `~/re/proview-cctv/acceptance/<sha>-<timestamp>/`:

| File | Content |
|---|---|
| `report.md` | the human summary: stage table, numbers, samples, dump path |
| `report.json` | the same data for a script; schema `proview-live-acceptance/1` |
| `server.log` | the server's own log for the run |
| `dump/frames.h264` | elementary stream from the first keyframe (H.264), for ffprobe outside the harness; a passthrough run writes the fragment stream as `dump/frames.mp4` instead |
| `dump/config.bin` | the AVCC/HEVC config record the socket sent |
| `dump/frames.tsv` | one row per dumped frame: kind, pts, bytes, seconds into the run |

The report names the framing it saw (`live.framing`): `framed` means each binary message starts with a
one-byte type (`1` config, `2` keyframe plus an eight-byte pts, `3` delta); `raw` means the socket
carried whole fMP4 boxes, and the keyframe count then comes from `/api/live/stats` alone.

Verify the dump outside the harness, because a byte count is not a picture:

```sh
ffprobe -v error -f h264 -count_frames \
  -show_entries stream=codec_name,width,height,nb_read_frames \
  ~/re/proview-cctv/acceptance/<run>/dump/frames.h264
```

The report keeps `frames`, `keyframes`, `bytes`, `fps_avg`, `first_frame_s`,
`longest_gap_s` and one sample row every ten seconds by default. `fps_avg` is
the average over the whole run; a stream that stalls in the middle still shows a
plausible average, which is why the gap column and the `stalls` list exist.

### Where the keyframe count lives

The ticket from `GET /api/stream` carries `kind`, `url`, `expires_in` and
`source`. The per-stream counters the server keeps, including `keyframes`,
`packets` and `bytes`, are served by `GET /api/live/stats`. The harness reads
them there and records the value under `live.server_keyframes`, then prefers the
socket's own count under `live.keyframes_ws`. Both appear in `report.md`; a
report that shows a socket count of zero and a server count above zero means the
framing changed on the wire, not that the camera stopped.

## Limits the harness enforces on itself

1. One live session at a time. The relay lock is taken before the server starts
   and released after it stops.
2. No retry against a busy relay. A vendor "busy" answer fails the run; the next
   run is a new run with a new timestamp.
3. No crate edits. The harness lives in `tools/acceptance/`, builds the branch
   under test, and never patches the code it measures.
4. No credentials in the report. The sign-in material is read from the secrets
   file and never written; the report names the file, never its contents.

## Recording the outcome

A run that matters gets one row in `docs/performance.md` under
"Live cloud acceptance", quoting the run directory so the numbers stay
reproducible. A run that fails gets a line naming the stage, because a failure
with a stage is worth more than a success without numbers.
