# Changelog

Every release is a tag on `main`; each one publishes 8 server binaries, the desktop
installers and a `SHA256SUMS` file from the same workflow. Format follows
Keep a Changelog; this project uses `0.MINOR.PATCH` and does not promise API
stability before 1.0.

## 0.3.0 — 2026-10-27

A new interface: one window onto one camera. `DESIGN.md` describes it; the study
behind it is `docs/design/redesign-v1/proview-redesign-v1.pdf`.

### Changed

- The picture fills the window. Live is the camera picture, one caption line under
  it (camera name, the state in words, a clock) and one row of controls; the
  device rail, the Camera controls panel, the View/Quality segmented controls and
  the source badge are gone.
- The camera name in the top bar is the camera picker: a menu of every camera
  (each device channel is one camera) with its status, keys 1–9, Refresh and
  Add a camera.
- Lens and quality are menus under the picture. A dual-lens camera offers Both
  lenses, Top and Bottom, named by role when the server reports which lens moves.
- Pan and tilt moved into a pad over the picture (Move, or V), with the device's
  presets by name. Controls the camera cannot do stay disabled with the server's
  reason as their tooltip.
- The capability report with the device's evidence, the camera id, connection and
  lens layout moved to Settings → This camera, together with the white light.
- Settings is a full screen with sections (Playback, This camera, Connection,
  Configuration, Server), a save bar that counts unsaved fields, Discard, Ctrl+S,
  and the restart prompt in the bar after a save that needs one. Sign out moved
  to Connection; "Turn the picture 180°" became a Playback preference.
- New colour tokens (graphite, one meaning per colour), IBM Plex Sans and Mono
  bundled in `web/fonts/` under SIL OFL 1.1 and served at `/fonts/*`, so the
  desktop app looks the same on WebView2, WKWebView and WebKitGTK and needs no
  network for fonts.

### Added

- Keyboard: 1–9, G, F, M, T (hold to talk), R, S, L, Q, V, arrows, Space, `,`,
  Esc, and `?` for the list.
- Hold-to-talk (pointer, Space/Enter on the button, or T) with a microphone level
  meter from an `AnalyserNode`. It is enabled only when the device reports
  two-way audio as supported; today's server reports it blocked because it has no
  capture path to the camera's talk channel.
- Every-camera view: up to four cameras, each tile its own stream and its own
  failure with Try again, and one summary line.
- States in words: connecting, frozen with the time of the last frame, failed
  with the server's answer and Try again, camera offline, this computer offline
  (banner, stale picture), session ended (sign-in says when and resumes the same
  camera).
- `tests/web/ui.test.mjs`: headless Chrome over the DevTools protocol, no new
  dependency. Covers the camera menu with the Popover API and with the API
  removed (roving focus, wrap, Escape returns focus, outside click), shortcuts
  and their text-field guard, the `?` sheet, and hold-to-talk with a fake
  microphone. CI runs it in the `check` job.
- `tools/contrast.py` reads the new tokens, composites alpha tokens over their
  real background, and runs in CI; 52 of 52 pairs pass.

### Fixed

- Menus work without the Popover API: when `showPopover` is missing the app uses
  a hidden-toggle with an outside-pointer handler and the same keyboard handling.
- The focus ring shows on keyboard focus only (`:focus-visible`), 2 px with a 2 px
  gap; every control is at least 36×36 px, 44 px under `pointer: coarse`;
  `prefers-reduced-motion` turns the motion tokens to 0.

## 0.2.6 — 2026-10-27

Cloud account login, in Rust, from `main`.

### Added

- The signed v3 cloud envelope ships: `crates/core/src/cloud/envelope.rs`
  reproduces the vendor `GetEncryptStr` scramble, the per-request `tsid` and
  `sig`, the AES-128-ECB body key and the sealed reply, and
  `crates/core/src/provider/cloud.rs` drives it end to end — email + password →
  `accessToken` → platform exchange → device token → device list. `POST
  /api/login` and `GET /api/devices` are the routes that use it.
- `PROVIEW_APP_UUID`, `PROVIEW_APP_KEY` and `PROVIEW_APP_SECRET` under
  Settings, Configuration, Cloud, with the missing-key notice naming all three.
- `crates/core/tests/envelope.rs` and `fixtures/cloud-envelope.json` pin the
  derivation to the 27 `GetEncryptStr` rows captured from the vendor library and
  to one captured login body.

### Changed

- `protocol.md` sections 3A and 3B record the derivation and the shipped client,
  `performance.md` adds the per-step cloud login numbers, and `settings.md`
  lists the three identity keys.
- README status, `cloud-app-surface.md`, `parity.md`, `providers.md` and
  `mobile-lab.md` say the login is implemented and exercised here, instead of
  "proven in the vendor SDK by an out-of-tree harness".

## 0.2.5 — 2026-10-27

### Fixed

- `proview-server --version` (and `-V`) printed nothing and started the server
  instead. The option parser treated every option as one that takes a value, so a
  bare flag never matched and `--version` fell through to `main`. It now prints
  `proview-server <version>` and exits, with a unit test for the flag shape.

### Changed

- The whole documentation set was brought to this release: README rewritten
  (status, feature table, Settings-first configuration, the cookie secret a
  release build needs, honest cloud login status), `CHANGELOG.md` added,
  `protocol.md` section 3A records what the vendor SDK harness proved,
  `cloud-app-surface.md` / `parity.md` / `providers.md` say plainly that the
  login envelope is proven but not integrated, `deploy.md` no longer claims
  credentials never touch disk, `install.md` uses the real asset names, and
  `settings.md`, `performance.md`, `webview-matrix.md`, `mobile-lab.md` and
  `DESIGN.md` match the code. The v0.2.4 tag predates this pass.

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
