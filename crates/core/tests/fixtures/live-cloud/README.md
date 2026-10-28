# Live cloud media fixtures

The body sequence the cloud relay delivers on DVRIP msgid 1412, captured from the
camera over the RPS tunnel on 2026-10-28 and re-wrapped here with synthetic pixel
content so a public repository never carries camera imagery.

## Layout, as measured

| class | size | content |
|---|---|---|
| `f9` | 16 B | GOP marker before a keyframe |
| `fc` | 8192 B | keyframe head: `00 00 01 fc` + `53 0c 20 44` + u32 calendar stamp + u32 declared payload length, then Annex-B HEVC |
| none | 8192 B | headerless continuation of the same keyframe until `declared` is satisfied |
| `fd` | 8 B + payload | delta: `00 00 01 fd` + u32 declared, then one whole Annex-B access unit |
| `fa` | 328 B | audio: 8-byte header + 320 B A-law, measured on the 216 bodies of the capture; `LiveDemux` drops it and counts it under `padding` |

When `OPMonitor Start` carries `VEKEY1`, the first 128 payload bytes of every
keyframe are XOR-masked; deltas stay in the clear. `LiveCipher` derives the
128-byte keystream from that material, so a keyed capture demuxes back to the
unmasked elementary stream byte for byte.

## Files

| file | content |
|---|---|
| `source-1280x720.hevc` | synthetic HEVC, 8 access units (2 IRAP), 74 322 B |
| `cloud-keyed-1280x720.bodies` | 22 bodies, 80 530 B of body bytes (80 618 B on disk): masked keyframes, plain deltas, audio, markers, and a trailing keyframe the capture cut in half |
| `cloud-plain-1280x720.bodies` | 14 bodies, 54 214 B of body bytes (54 270 B on disk): the same opening frames with no mask, the shape a capture without `VEKEY1` produces |
| `make.py` | regenerates both `.bodies` files from the `.hevc` source |

Every `.bodies` file is a flat run of `[u32 little-endian length][body]`.

The fixture key material is `zha4084-live-cloud-fixture-material-0001`, pinned in
`crates/core/tests/live_cloud_demux.rs`. It is a test value, not a device secret.

## What the real capture measured

Taken before the pixel content was replaced, so the numbers describe the device
rather than the fixture:

| run | bodies | elementary stream | ffprobe |
|---|---|---|---|
| Main stream, `VEKEY1` set, 9 s | 456 (3 `f9`, 4 `fc`, 128 continuations, 216 `fa`, 105 `fd`), 1 190 157 B | 1 085 605 B | `hevc`, 2304x2592, 12 fps, 108 frames |
| Main stream, no `VEKEY1` | 478 bodies, 1 370 708 B | decodes straight from the demux | `hevc`, 2304x2592 |

The masked head that run carried was
`3b 06 fe 89 d4 19 45 3a 11 2d a0 da 34 1f 92 9f` on the wire; unmasking it with
the run's own `VEKEY1` recovered `00 00 00 01 40 01 0c 01`, a VPS. `ffmpeg -f hevc`
consumed the demuxed stream with exit code 0 and an empty error log.

## Regenerating

```
ffmpeg -f lavfi -i testsrc2=size=1280x720:rate=12 -frames:v 8 \
  -c:v libx265 -x265-params 'keyint=4:min-keyint=4:scenecut=0:aud=1:info=0:repeat-headers=1:crf=34' \
  -pix_fmt yuv420p -f hevc source-1280x720.hevc
python3 make.py --source source-1280x720.hevc --out .
```
