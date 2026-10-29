# Fisheye dewarp: the projection model recovered from libfisheye.so

Provenance: `libfisheye.so`, ELF64 aarch64, stripped, 146832 bytes, SHA-256
`5ab93e63b4917d60379eb866db84a1453b460ab8caf54738dda01c0b0ffff7ad`, taken from
`config.arm64_v8a.apk` (SHA-256 `67d2026e3dce3f979eaa64b556d746c437b715848e57e96fec8afae10e8921fd`)
inside the 1.0.4 xapk. Static reading only: `aarch64-linux-gnu-objdump` 2.46.1, `readelf`
2.46.1, `strings`, Python 3.15. Artefacts live in `~/re/proview-cctv/ZHA-4015`.
The library is never executed on this host; the APK and the `.so` are data.

Everything below is read from the disassembly or from the shader source embedded in
`.rodata`. Anything recorded from inference is marked.

## 1. Call chain

| Layer | Artifact | Evidence |
|---|---|---|
| Java | `com.vatics.dewarp.GL2JNILib` loads `fisheye` and declares 22 natives | `GL2JNILib.java` |
| JNI | `libFunSDK.so` exports `Java_com_vatics_dewarp_GL2JNILib_*` (22 symbols, 0xde9464..0xdec000) | `objdump -T libFunSDK.so` |
| Engine | `libfisheye.so` exports `Fisheye_*` (18), `CFisheyeMap` methods (58), `FisheyeGL_*` (35), `SU_*`, `OSCriticalSection_*`, a SHA-1 helper | `objdump -T libfisheye.so` |

The JNI glue only packs options: `setMountType` writes the value at option offset 76 and
sets the mask to `0x40`; `setDewarpType` writes offset 80 and mask `0x80`
(`Java_com_vatics_dewarp_GL2JNILib_setMountType` at 0xdeae74, `..._setDewarpType` at
0xdeaf10). `Fisheye_SetOption` (0x5ab8) forwards to `Fisheye_InternalSetOption` (0x69dc),
which dispatches on that mask.

## 2. Calibration the camera supplies

`SDK_FishEyeFrameSW` (smali `com/lib/sdk/struct`), 14 bytes (offsets 0..13):

| Offset | Field | Type |
|---|---|---|
| 0 | `version` | u8 |
| 1 | `lensType` | u8 |
| 2 | `centerOffsetX` | i16 |
| 4 | `centerOffsetY` | i16 |
| 6 | `radius` | i16 |
| 8 | `imageWidth` | i16 |
| 10 | `imageHeight` | i16 |
| 12 | `viewAngle` | u8 |
| 13 | `viewMode` | u8 |

`lensType` values (`SDK_FishEyeFrameSW$FISHEYE_LENS_TYPE_E`): 0 general, 1 vr360, 2 lvr360,
3 vr180, 4 dual vr360, 5 dual vr180, 6 vr180 pro, 7 general single pip.
Mount values (`SDK_FishEyeParam$FISHEYE_APP_TYPE_E`): 0 ceiling, 1 table, 2 wall.
Scene values (`FISHEYE_SECENE_E`, the `setDewarpType` argument): 0 orig, 1 rect, 2 p180 all,
3 p180 one, 4 p180 two, 5 p360 full, 6 p360 sepe, 7 p360 half, 8 rop-r, 9 rorr-r, 10 rrrr-r,
11 rrp-r, 12 p360-fe.

The offsets are relative to the image centre; the map object stores a pixel centre
(`CFisheyeMap::SetInfo_InputImageCenter` at 0xb68c) and a pixel radius
(`SetInfo_InputImageRadius` at 0xb6d0). That the two halves are added is inference; the
struct alone carries only offsets.

## 3. Lens curve: radius to polar angle

`Fisheye_GenAndSetLensInfo(handle, double const* table, u32 count, double step)` at 0x71a0
copies the vendor radial table and derives one scale:

    scale = 2 * xd[count - 1] / pi          (0x7234 divides by the pi constant at 0x7470)

`CFisheyeMap::GetThetaXYFromFlatXY(int x, int y, float* yawDeg, float* pitchDeg)` at 0x103d0
turns an image point into angles:

    dx = x - centerX;  dy = y - centerY;  r = hypot(dx, dy)
    phi = atan2(|dy|, |dx|)                    (then + 2pi, fmod 2pi)
    if table present:
        j = index with xd[j] <= r < xd[j + 1]
        theta = step * (j + (r - xd[j]) / (xd[j + 1] - xd[j])) * pi / 180
    else:
        theta = r / scale
    yaw   = atan(tan(theta) * cos(phi))         (atan2 branch when theta == pi/2)
    pitch = acos(cos(theta) / cos(yaw))
    both are returned in degrees, negated on the low side of the centre

Constants that confirm the arithmetic: `0x10738` = pi/2, `0x10740` = 2pi, `0x10748` = pi,
`0x1074c` = 180, `0x10750` = 57.29578 (180/pi). The no-table branch is the equidistant lens:
`theta = r / scale`, so `scale = radius / maxTheta`. For a 180 degree lens
`maxTheta = pi/2` and `scale = 2 * radius / pi`, which is the vendor scale formula with
`xd[count - 1] = radius`.

`CFisheyeMap::CalibrateThetaXY(float a, float b)` at 0x113a0 is the reverse-side helper:

    theta = pi/2 - |asin(1 / hypot(a, b)) - atan(-|a| / b)|
    result = trunc(theta * 1000) / 1000            (0x11428 = pi/2, 0x1142c = 1000)

so the vendor quantises that calibration to milliradians.

## 4. Equirectangular convention

The panorama builder and the shader share one convention: the image plane is the horizon
and the lens axis is the pole.

    bearing   = atan2(y, x)                        -> u = mod((pi/2 - bearing) / 2pi, 1)
    elevation = atan2(z, hypot(x, y))              -> v_texture = elevation / pi + 0.5

Two spaces have to be distinguished, and mixing them is what produced the mirror this
model had until round 3:

- `v_texture` is a GL texture coordinate: it grows toward the last uploaded row.
- A raster pano (`Map`, BMP, `<canvas>`) counts rows downward from the top, so its
  `v_raster = row / height` equals `v_texture` only if the first row is `v_texture = 0`.

The vendor's own display path decides how those meet. `FisheyeGL_BuildQuadsList`
(0xa1f0) writes four vertices with the pairing position-to-texcoord
`(-1,-1)->(0,0)`, `(-1,1)->(0,1)`, `(1,1)->(1,1)`, `(1,-1)->(1,0)`: no flip on either axis,
and the vertex shader passes `a_vTexCoord` straight into `v_vTexCoord`. Read with REA
6.7.0 / Ghidra 12.1.4 (`rea.open_binary`, `rea.procedure_pseudo_code`), file SHA-256
`5ab93e63...f7ad`. So a pano texture whose row 0 holds `v_texture = 0` is drawn with that
row at the bottom of the screen, and column 0 at the left. The vendor's own `u` therefore
runs opposite to image bearing with no compensating flip in the quad, which is why a
literal copy of the texture formula reads as a horizontal mirror next to the PTZ path.

What the implementation does, and why: the raster pano uses

    u = 0.25 + bearing / 2pi        v_raster = 0.5 + elevation / pi

so a marker at image bearing +30 degrees lands at u = 0.3333 and one at -30 degrees at
u = 0.1667, i.e. increasing bearing runs to the right, the same direction the PTZ path
resolves. The sign of `u` therefore differs from a literal copy of the shader's texture
coordinate, and that is deliberate: with the literal sign the equirect map and the PTZ map
of the same lens disagreed, and the equirect one came out mirrored (det < 0, 7832 of 7832
interior points at 180x90). The orientation is pinned by
`every_map_keeps_the_source_handedness`, which requires the discrete Jacobian of the source
map, `det[s(c+1,r) - s(c,r), s(c,r+1) - s(c,r)]`, to be positive at every interior point of
`Map::equirect`, all three `Map::equirect_mounted` variants and `Map::ptz`; at 180x90 that
is 7832 points for equirect, 7832 for ceiling, 7654 for table, 7832 for wall and 14608 for
ptz. Measured in a y-up basis all five are negative, which is the same statement once the
basis flips.

What stays unproven is narrower than the quad: the row order of the buffer the vendor
uploads as its pano texture (whether row 0 is the nadir or the zenith) and whether the
application flips the rendered output before it reaches the screen. Neither changes the
horizontal handedness the quad evidence settles.

Source coordinates use the pixel-area convention: column `c` covers `[c, c+1)`, so the
geometric centre of a 512 px frame is 256.0 and `RgbFrame::sample` subtracts 0.5 before
interpolating. `fixtures/fisheye/make.py` and `apps/web/src/live/fisheye.ts` use the same centre.

`FisheyeGL_BuildFullPanoMap` (0xa2c8) sizes the pano from the output height and the field
of view (`width = height * aspect / 360`, constant `0x43b40000` = 360.0), and
`CFisheyeMap::BuildFullPanoMap` (0xc9a4) inverts latitude with `atan(sinh(v))`
(0xca64 `sinhf`, then `atanf`), the Mercator variant of that path.

The GLSL source at `.rodata` 0x134e8 states the same mapping for the interactive path:

    vec2 GetSourcePoint(vec2 texCoord) {
        vec2 vDst = (texCoord * 2.0 - 1.0) * u_vOAR * u_fRatio;
        vec3 vPT3D = Transform(vDst.x, vDst.y);
        float theta = 1.570796327 - atan(vPT3D.y, vPT3D.x);
        float sigma = atan(vPT3D.z, sqrt(vPT3D.x*vPT3D.x + vPT3D.y*vPT3D.y));
        newPoint.x = mod(theta * 0.159154943, 1.0);
        newPoint.y = ((sigma * 0.318309886 + 0.5) * u_vH.x - u_vH.y) / u_vH.z;
    }

with the virtual-camera ray (same shader, function `Transform`):

    float a = x*x + y*y;
    float z = (a * u_fCamZ - sqrt(a - a*u_fCamZ + 1.0)) / (a + 1.0);
    return u_mat * vec3((u_fCamZ - z) * x, (u_fCamZ - z) * y, z);

`u_fCamZ` is the unified-sphere parameter: 0 degenerates to the pinhole ray
`(x, y, -1)/sqrt(1+a)`, 1 to the stereographic inverse `(2x, 2y, 1-a)/(1+a)`.
`u_mat` is the pan/tilt matrix built in `FisheyeGL_UpdatePanoramaShaderParameters` (0xa4c4)
from the pan (offset 84), tilt (92) and roll (100) floats, stored at offsets 848..880 as a
3x3. The remaining uniforms are the output aspect (`u_vOAR`), the zoom ratio (`u_fRatio`
= 100.0 / zoom, `FisheyeGL_UpdateScaleRatio` 0x8fa8) and the vertical crop window `u_vH`.

## 5. What the implementation does

`crates/core/src/fisheye.rs` implements exactly the above:

| Rust | Vendor source |
|---|---|
| `Curve::Equidistant` + `Lens::scale` | `GetThetaXYFromFlatXY` no-table branch, `Fisheye_GenAndSetLensInfo` scale |
| `Curve::Table` | the `xd[]` interpolation of `GetThetaXYFromFlatXY` |
| `Direction::equirect` / `from_equirect` | `GetSourcePoint` bearing/elevation convention |
| `View::ray` | shader `Transform`, with `z` flipped so the optical axis is +Z |
| `View::rotated` | `u_mat` pan then tilt |
| `Map::equirect`, `Map::ptz` | `BuildFullPanoMap`, `Fisheye_PanTiltToInVPicturePoint` (0x6278) |
| `Calibration::lens`, `LensType`, `MountType` | `SDK_FishEyeFrameSW`, `FISHEYE_LENS_TYPE_E`, `FISHEYE_APP_TYPE_E` |
| `RgbFrame::warp` | the GL draw with a source map, bilinear instead of the GPU sampler |

### Mount type

The vendor exposes `SetInfo_UpsideDown` (a boolean flip of the map) and the three-axis
`SetInfo_RotateThetaX/Y/Z`, and the SDK mount code (ceiling 0, table 1, wall 2) is what
selects the combination. Our map applies that as a pitch of the sphere before projection:
ceiling 0 degrees, table 180, wall 90 (`MountType::pitch_deg`, `Map::equirect_mounted`,
`u_mountPitch` in the shader). Which exact vendor rotation each mount code maps to is
inference, because the option page carries only the code, but the two knobs it drives are
named in the disassembly and the effect is visible: with `mount=table` the pano centre
stops sampling the lens centre and the covered half moves to the other side.

### Live frame

`apps/web/src/live/fisheye.ts` carries the same math into WebGL2, and `FisheyeOverlay.vue`
puts its canvas over the application's own `<video id="video">` element and dewarpes every
animation frame. It is opt-in through the URL (`?fisheye=equirect|ptz`, plus `mount`, `pan`,
`tilt`, `fov`) and publishes itself as `window.ProviewFisheyeLive` (its `stop()` removes the
canvas), so it changes nothing until asked. `e2e/tests/fisheye.spec.ts` renders a
coordinate-coded 512x512 lens through the shader and compares every output pixel with
`/api/fisheye/map` for an equirect and a ptz view.
`crates/server/src/routes/fisheye.rs` exposes the model document, a source map and a
fixture render. Two limits are deliberate and
recorded in the parity row: nothing in the Live screen (`apps/web/src/app/`) calls the renderer by default, so no live
frame reaches either path, and `MountType` is decoded but does not rotate the map, so
ceiling, table and wall mount produce the same panorama.

## 6. Measured on the fixture

`fixtures/fisheye/fisheye-grid-512.ppm` (512x512, centre 256,256, radius 240, 12 spokes
every 30 degrees, 6 rings every 15 degrees), generated by `fixtures/fisheye/make.py` with an
independent forward projection. Measured by `cargo test -p proview-core --lib fisheye` and
the route test in `crates/server/tests/fisheye.rs`:

| Check | Result |
|---|---|
| Worst spoke bearing error, dewarped to 720x360 equirect | 0.25 deg (0.5 px) |
| Worst ring elevation error | 0.25 deg (0.5 px) |
| Pixel to direction to pixel round trip, 360 bearings x 47 radii | 9.2e-5 px |
| Radial table interpolation, 91 samples, 900 angles | 1.1e-5 deg |
| Valid pixels, 720x360 equirect of a 90 degree lens | 129600 of 259200, the covered hemisphere |
| Valid pixels, 640x360 ptz at 90 degree fov, pan 30 | 227520 of 230400 (98.75%) |
| Dewarp request 720x360 through the route, release build | 13.10 ms, median of 3, 777654 B BMP |
| Dewarp request 2048x2048 at the 4194304 px cap, release build | 202.63 ms, median of 3, 12582966 B BMP |
| Source map request 256x128 JSON, release build | 4.00 ms, median of 3, 547444 B |
| Live overlay on the app's own video element, headless Chromium | 172848 of 345142 px painted (50.08%, the covered hemisphere) at 554x623 |
| Mount coverage, 360x180 map | ceiling 32400 px, table 32404, wall 32404; the table map trades the centre pixel for its mirror |

The bearing and ring residuals are one equirect column and one row: 720 columns over 360
degrees is 0.5 deg, 360 rows over 180 degrees is 0.5 deg, and both land on 0.25 deg, i.e.
half a texel. They are rasterisation, not model error, which the round trip (9.2e-5 px) and
the table check (1.1e-5 deg) pin four orders of magnitude lower.

The release numbers come from the built binary serving the embedded fixture with no
`fixtures/fisheye` directory on disk, which is also the smoke test for the release
artifact: the request still returns 200 and a full BMP.
