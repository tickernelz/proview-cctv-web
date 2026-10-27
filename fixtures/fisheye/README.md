# Fisheye fixture

`fisheye-grid-512.ppm` is a 512x512 binary PPM drawn with an equidistant forward
projection: centre (256, 256), radius 240 px, `theta = radius_px * maxTheta / 240` with
`maxTheta = 90 deg`. It carries 12 spokes at bearings 15, 45, ... 345 degrees (inner
radius 0.10 R, outer 0.96 R, 2.2 px half width) and 6 rings at theta = 15, 30, 45, 60, 75
and 90 degrees (1.6 px half width), drawn over a dark background with 4x4 supersampling.

The 15 degree phase keeps every spoke off the pano seam at u = 0 and off the +-180 degree
column; a spoke sitting on the seam split between the first and last column and skewed the
median by half a column.

The pattern is symmetric under reflection, so this file cannot by itself detect a mirrored
equirect. `equirect_matches_the_vendor_orientation` and
`equirect_places_markers_on_the_columns_their_bearing_implies` in
`crates/core/src/fisheye.rs` pin the handedness with asymmetric markers instead.

The geometry is independent of the Rust implementation: `make.py` projects forward, the
Rust tests project backward and compare the result against the bearings and elevations the
file was drawn with.

Regenerate and verify:

    python3 fixtures/fisheye/make.py fixtures/fisheye/fisheye-grid-512.ppm
    sha256sum fixtures/fisheye/fisheye-grid-512.ppm

The committed file has SHA-256
`dbafcb07dda754a5ce3cbf003e0f224d464867f4b1ce493d7f9eec40dfb6100a`.
