# Fisheye fixture

`fisheye-grid-512.ppm` is a 512x512 binary PPM drawn with an equidistant forward
projection: centre (256, 256), radius 240 px, `theta = radius_px * maxTheta / 240` with
`maxTheta = 90 deg`. It carries 12 spokes every 30 degrees (inner radius 0.10 R, outer
0.96 R, 2.2 px half width) and 6 rings at theta = 15, 30, 45, 60, 75 and 90 degrees
(1.6 px half width), drawn over a dark background with 4x4 supersampling.

The geometry is independent of the Rust implementation: `make.py` projects forward, the
Rust tests project backward and compare the result against the bearings and elevations the
file was drawn with.

Regenerate and verify:

    python3 fixtures/fisheye/make.py fixtures/fisheye/fisheye-grid-512.ppm
    sha256sum fixtures/fisheye/fisheye-grid-512.ppm

The committed file has SHA-256
`012999c57e64ea4d72744dd8e8b2236902cfd712d0474b44f7419e5593fa8a8a`.
