# Lens contract — two lenses in one stream

The camera sends one RTSP stream whose frame holds two pictures stacked
vertically. There is no second channel and no second URL: the split is a crop.
This document is the contract between the server and any client that draws the
live view, and it records which piece of evidence decided each field.

Types live in `crates/core/src/lens.rs` (detection, crop arithmetic) and
`crates/core/src/model.rs` (wire payload). The route is `GET /api/lens` in
`crates/server/src/lens_route.rs`.

## 1. Measured facts

Device: PROVIEW IPC `192.168.0.100` (`XM535_X6E-WEQ_8M`), RTSP 554, measured
2026-10-26.

| What | Value | How it was measured |
|---|---|---|
| Main stream SDP | `2304x2592`, H.265 | `DESCRIBE` on `/user=&password=&channel=1&stream=0.sdp?real_stream` |
| Sub stream SDP | `640x720`, H.265 | same, `stream=1` |
| Channel/stream combinations | `channel=0..3` × `stream=0..1` return an identical SPS fingerprint | byte-compare of the parameter sets |
| Stack arithmetic | `2304x2592 = 2 × (2304x1296)`, `640x720 = 2 × (640x360)` | both halves are 16:9 |
| Which half moves | bottom half shifts, top half does not | ONVIF `ContinuousMove` (PanTilt x=0.15, `PT0.6S`) then `GotoPreset`; per-half translation estimate on lossless PNG frames: bottom `(-608,-64) px`, top `(0,0) px` |

Noise floor for the same comparison without a move: both halves `(0,0) px`.
Aligned PSNR drops on both halves (33 → 27 on the top), which is the shared 3A
exposure change, not motion: shift estimation is the movement detector, raw PSNR
is not.

## 2. Lens-count evidence, in priority order

1. **Device capability, when reachable.** `com/lib/sdk/bean/SystemFunctionBean$OtherFunction`
   carries `MultiLensTwoSensor` and `MultiLensThreeSensor` (fields 84–85),
   plus `SupportScaleTwoLens` / `SupportScaleThreeLens` (172–173) and
   `SupportGunBallTwoSensorPtzLocate`. `MonitorActivity` stores the first two
   per channel as `"MultiLensTwoSensor" + channelId` after a
   `JsonConfig.SYSTEM_FUNCTION` reply; `WbsMonitorActivity` does the same.
   The native renderer in `libvrsoft.so` has the matching model
   (`__init_two_lenses_in_one_model`, `DEFAULT_ZOOM_LENS_DIV_POINT`,
   `__model_two_lenses_getScreen`) and the JNI entry points
   `multiGetLensCount` / `multiSwitchLens` / `setTwoLensesDrawMode`.
   `getTwoLensesScreen() == 3` is what makes the app switch to the two-lens
   draw mode.
2. **Frame geometry, as the fallback.** A measured frame is a stack when
   `height / width ≈ n × 9/16` for `n = 2` or `n = 3` (tolerance 0.02).
   `2304/2592` and `640/720` give `1.125` exactly, which is two 16:9 frames.
   Anything else counts as one lens.
3. **Unknown.** No frame size and no capability means no lens count at all; the
   payload then carries `lensCount: 0` and an empty `lenses` array. The client
   must not invent lenses.

`TR_Common_*` was searched as a candidate source and is a dead end: the build
has 18 such strings and none of them concerns lenses (`TR_Common_Change_Chn`,
`TR_Common_privacy_permission*`, `TR_Common_remind_*`).

The capability path is implemented and reachable through
`PROVIEW_LENS_CAPABILITIES` (comma-separated APK field names). It is not filled
from the device yet because the DVRIP config channel still fails (`ret 203`,
ZHA-3912); until then the runtime default is the geometry rule and the payload
says so in `countSource`.

## 3. Which lens pans and tilts

`SupportGunBallTwoSensorPtzLocate` says the device pairs a fixed gun lens with a
movable ball lens, but the APK never binds a lens index to "movable". The PTZ
measurement in §1 settles it for this device family: the **bottom** half moves.

Therefore `lenses[i].ptz` is a per-lens boolean:

- two-lens layout: index 0 `ptz: false`, index 1 `ptz: true`, `ptzSource: "measured_default"`;
- one or three lenses: every `ptz: false`, `ptzSource: "unknown"`;
- `PROVIEW_LENS_PTZ=<index>` overrides the index and sets `ptzSource: "operator"`.

Consequences the client can rely on: the crop of a `ptz: false` lens never moves
while a PTZ command runs, so its viewport may be cached or reused more
aggressively than the `ptz: true` one.

## 4. Wire payload

The route is authenticated like every other `/api` route (`pvw_session` cookie).
Query: `device` (required), `channel` (default `1`), `quality` (`main` | `sub`,
both when omitted), `width` + `height` (client hint, only used when the server
cannot read the parameter sets).

Fields:

| Field | Type | Meaning |
|---|---|---|
| `deviceId` | string | device the view belongs to |
| `channel` | number | vendor channel (this camera answers on all of `0..3`) |
| `streams[]` | array | one entry per probed quality |
| `streams[].quality` | `"main"` \| `"sub"` | sub stream is the cheaper one (640x720) |
| `streams[].codec` | string | `H264` / `H265` from the SDP |
| `streams[].width`, `.height` | number | `0` when unknown |
| `streams[].sizeSource` | `sdp_sps` \| `client_hint` \| `unknown` | where the size came from |
| `lenses[]` | array | empty when the count is unknown |
| `lenses[].index` | number | 0-based, top to bottom |
| `lenses[].label` | `full` \| `top` \| `middle` \| `bottom` | stable token, not display text |
| `lenses[].crop` | `{x,y,w,h}` | normalised 0..1 rectangle inside the decoded frame |
| `lenses[].ptz` | boolean | this lens pans/tilts |
| `lensCount` | number | resolved lens count |
| `countSource` | `capability` \| `geometry` \| `unknown` | which rule decided `lensCount` |
| `stack` | `vertical` \| `single` | how the frame is split |
| `cropsVerified` | boolean | `countSource` and the frame geometry agree |
| `capabilities` | object | the raw APK flags this decision saw |
| `ptzSource` | `measured_default` \| `operator` \| `unknown` | where `lenses[].ptz` came from |

Live payload from the camera above, both streams, geometry path
(`GET /api/lens?device=local&channel=1`, 457 ms, two `DESCRIBE` handshakes):

```json
{
  "deviceId": "local",
  "channel": 1,
  "streams": [
    { "quality": "main", "codec": "H265", "width": 2304, "height": 2592, "sizeSource": "sdp_sps" },
    { "quality": "sub", "codec": "H265", "width": 640, "height": 720, "sizeSource": "sdp_sps" }
  ],
  "lenses": [
    { "index": 0, "label": "top", "crop": { "x": 0.0, "y": 0.0, "w": 1.0, "h": 0.5 }, "ptz": false },
    { "index": 1, "label": "bottom", "crop": { "x": 0.0, "y": 0.5, "w": 1.0, "h": 0.5 }, "ptz": true }
  ],
  "lensCount": 2,
  "countSource": "geometry",
  "stack": "vertical",
  "cropsVerified": true,
  "capabilities": {
    "multiLensTwoSensor": false,
    "multiLensThreeSensor": false,
    "supportScaleTwoLens": false,
    "supportScaleThreeLens": false,
    "supportGunBallTwoSensorPtzLocate": false
  },
  "ptzSource": "measured_default"
}
```

Same call with
`PROVIEW_LENS_CAPABILITIES=MultiLensTwoSensor,SupportScaleTwoLens,SupportGunBallTwoSensorPtzLocate`
and `quality=main`, which is the capability path taking over
(`countSource: "capability"`, and `cropsVerified` stays true because geometry
agrees):

```json
{
  "deviceId": "local",
  "channel": 1,
  "streams": [
    { "quality": "main", "codec": "H265", "width": 2304, "height": 2592, "sizeSource": "sdp_sps" }
  ],
  "lenses": [
    { "index": 0, "label": "top", "crop": { "x": 0.0, "y": 0.0, "w": 1.0, "h": 0.5 }, "ptz": false },
    { "index": 1, "label": "bottom", "crop": { "x": 0.0, "y": 0.5, "w": 1.0, "h": 0.5 }, "ptz": true }
  ],
  "lensCount": 2,
  "countSource": "capability",
  "stack": "vertical",
  "cropsVerified": true,
  "capabilities": {
    "multiLensTwoSensor": true,
    "multiLensThreeSensor": false,
    "supportScaleTwoLens": true,
    "supportScaleThreeLens": false,
    "supportGunBallTwoSensorPtzLocate": true
  },
  "ptzSource": "measured_default"
}
```

## 5. How a client draws it

Pixel rectangle for a crop inside a decoded frame of `W × H`:
`x·W`, `y·H`, `w·W`, `h·H` (`crop_pixels` in `lens.rs`, the values are rounded).
For a 2304x2592 frame that is `0,0,2304,1296` and `0,1296,2304,1296`; for the
640x720 sub stream, `0,0,640,360` and `0,360,640,360`.

- **Combined view**: draw the frame untouched. `stack` tells the UI the two
  pictures inside it are vertical halves, so it can label them.
- **Single lens**: scale the video to the full panel, then hide the other half.
  With a panel of `w × h` and a video element of `w × 2h`: `top: 0` for lens 0,
  `top: -h` for lens 1. That is exactly the transform used to verify the
  fixture (see §6).
- **Which lens moves**: put the moving indicator on the lens whose `ptz` is
  true. Do not animate or refit the `ptz: false` panel on PTZ input.

The server never re-encodes a crop; the crop is client-side geometry over one
decoded frame. That keeps one upstream per device and one transcode per stream.

## 6. Fixtures

`fixtures/dual-lens/` holds a synthetic clip that reproduces the shape:
640x720, top half a fixed colour with a static block, bottom half a moving test
pattern. `make.sh` regenerates it with ffmpeg lavfi sources; the committed
files are byte-identical to its output. Measured on the committed file: mean
absolute difference between `t=0.1s` and `t=1.1s` is `0.23` in the top half and
`10.45` in the bottom half, so a swapped crop is visible in a single frame pair.

Rendered through the contract in headless Chrome, cropped with the CSS above:
the top panel is identical across the two timestamps (pixel difference `0.2`)
and the bottom panel changes (`9.49`).

`hevc-1920x1080.sps` is a 43-byte HEVC sequence parameter set generated by
`libx265`; it exists so the frame-size parser has a test vector that was written
by something other than the parser itself.

## 7. Not proven

- Three-lens stacking: the rule produces thirds, but no three-sensor device has
  been measured, so `cropsVerified` stays false without geometry that agrees.
- Whether every white-label build uses the same index order (top fixed, bottom
  movable). One device family was measured; `PROVIEW_LENS_PTZ` exists for the
  rest.
- Zoom and preset precision (residual 3 px of a 2304-px width after a restore),
  and the pan/tilt axis direction in image coordinates.
