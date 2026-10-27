# Changelog

Every release is a tag on `main`; each one publishes 8 server binaries, the desktop
installers and a `SHA256SUMS` file from the same workflow. Format follows
Keep a Changelog; this project uses `0.MINOR.PATCH` and does not promise API
stability before 1.0.

## 0.2.4 — 2026-10-27

Settings you can reach before signing in, and honest wording everywhere.

### Added

- Settings opens from the sign-in screen: a "Settings" link under the form shows
  the whole settings view with the Live and Grid tabs hidden until a session
  exists, and "Back to sign in" restores the form. This is the path a fresh
  install uses to set the cloud address, the camera address and the camera
  password.
- The desktop build shows **Restart the app** after a save that touched a key
  read once at startup (Tauri command `restart_app`, `withGlobalTauri`); a server
  build says to restart the process instead of offering a button that cannot work.
- `GET /api/settings` reports `exists`, so the view can say whether anything has
  been saved yet instead of claiming to have loaded a file that is not there.
- Camera-missing failures carry `code` (`device_host_missing`,
  `device_rtsp_missing`) so the UI branches on a code instead of matching copy.

### Fixed

- Consecutive saves no longer drop earlier values. The store is interior-mutable
  and each POST merges into what is already stored; previously every save rewrote
  the file from a clone taken before the previous write. Regression tests:
  `a_second_save_keeps_the_first`, `the_view_says_whether_a_settings_file_exists`.
- No user-facing message names a `PROVIEW_*` variable any more. The probe error,
  the fallback notice, the cloud login failure, the sample-data lookup and the
  missing-ffmpeg error now name the Settings screen and the group that owns the
  value. Tests assert the notices avoid raw keys instead of pinning wording.
- Settings labels and hints read as settings ("Camera address", "Cloud session
  lifetime", "ffmpeg program"), and the provider selects show words ("Vendor
  cloud", "This network", "Built-in sample data") instead of values
  `proview`/`local`/`fixture`.
- The device rail no longer appears next to Settings when nothing is signed in;
  it comes back with a session.

### Verified

- Empty data directory on a debug build: Settings opened from the sign-in screen,
  three consecutive saves merged into one file, the secret stayed masked
  ("stored, type to replace", `value: null` over the API), the restart row and the
  desktop button appeared, and the probe error read the new copy.
- `cargo test -p proview-core -p proview-server` (185 tests) and
  `cargo clippy -p proview-core -p proview-server --all-targets -- -D warnings`
  green on the tagged commit; the desktop smoke job builds the shell on Linux,
  Windows and macOS.

## 0.2.3 — 2026-10-27

### Fixed

- The provider defaults to the vendor cloud again; the fixture provider is opt-in
  with `PROVIEW_PROVIDER=fixture` (a build could start in sample data without
  saying so).
- Settings tests each use their own temporary directory, so a shared state
  directory cannot make them pass or fail together.

### Added

- Settings view over `GET/POST /api/settings`: every key with a label, a hint and
  a restart marker; the file is written atomically.
- README and `docs/parity.md` point at the settings view as the way to configure.

## 0.2.1 — 2026-10-27

### Added

- The desktop app writes a log next to its app data, and logs cloud failures
  there, so a failed sign-in leaves a trace a person can read.

## 0.2.0 — 2026-10-27

First release with the whole reachable surface.

### Added

- Live view from a LAN camera: RTSP `DESCRIBE` with Digest against a real IPC,
  RTP depacketize, H.265 detection, one transcode per stream, fMP4 over WebSocket
  into MSE; and a passthrough transport that hands HEVC access units to
  WebCodecs without re-encoding (`/api/live/stats` reports both).
- Dual-lens support for a stacked frame: `GET /api/lens` returns the crops, and
  the UI draws the split.
- Device control: DVRIP login with the vendor "sofia" hash, ONVIF movement and
  presets with an explicit timeout, snapshots, white light, picture flip, record
  mode, and a read-only ONVIF device/encoder/imaging/network view.
- Provider seam with `cloud` and `local`, a per-profile mode choice persisted in
  `mode.json`, and a documented fallback to the cloud that says why it happened.
- Vendor portal login and device list over the v2 portal, with the envelope and
  field digest recorded in `docs/protocol.md`.
- `GET /api/cloud/device-caps?sn=<serial>`: model, firmware, WAN address and the
  capability block from the app's own cloud, with no account session.
- Desktop shell keeps device credentials in the OS keychain, per profile.

### Changed

- `check` runs on `ubuntu-22.04`; pull request runs cancel their superseded
  predecessors; a `mobile-re` job boots an arm64 emulator on macOS runners.

## 0.1.0 — 2026-10-26

First tag: server skeleton, session cookie, fixture provider, DVRIP framing,
Annex-B and fMP4 packaging, SPS parse, and the protocol record.
