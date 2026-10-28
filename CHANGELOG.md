# Changelog

Every release is a tag on `main`; each one publishes 8 server binaries, the desktop
installers and a `SHA256SUMS` file from the same workflow. Format follows
Keep a Changelog; this project uses `0.MINOR.PATCH` and does not promise API
stability before 1.0.

## Unreleased

### Added

- Desktop alarm notifications and the tray. The desktop process polls the alarm
  centre on a timer (60 s by default, 15-3600, set in Settings) and posts one
  native system notification per new alarm — device, event and time — so the
  alarm arrives with the window minimized or closed to the tray. An alarm is
  keyed by device serial, channel and id, since an id is a one-second local
  timestamp that two cameras or two channels can share. What was announced is
  kept in `desktop-state.json` and expired by the age of the alarm, never by the
  number of entries, so an id cannot fall off a list and come back; an alarm
  older than the requested window is ignored whether or not it was announced, so
  the centre returning one again cannot re-post it. A run — the process starting
  with notifications on, or the switch turning on — treats what arrived before
  its first **successful** check as history and posts one summary instead of a
  toast per missed alarm; a check that fails before the window has signed in
  does not count, and a fresh install's baseline mark is saved even when that
  pass found nothing. A click focuses the window and opens that alarm in the
  Alarm messages sheet where the platform reports clicks, and a click on the
  summary opens the sheet with no alarm marked. The tray carries Open, Live and
  Quit; a left click on it focuses the window, and a second launch focuses the
  running window instead of starting a second server. Closing the window keeps
  the app running (default), minimizing hides it (off by default), a macOS Dock
  click brings it back, and the first close announces once that the app is still
  in the tray. A tray that cannot be built — including a Linux box without the
  AppIndicator library — turns both behaviours off rather than hiding a window
  with no way back. Four new keys, `PROVIEW_DESKTOP_ALARM_NOTIFY`,
  `PROVIEW_DESKTOP_ALARM_INTERVAL`, `PROVIEW_DESKTOP_CLOSE_TO_TRAY` and
  `PROVIEW_DESKTOP_MINIMIZE_TO_TRAY`, sit under Settings → Configuration →
  Desktop, are read by the shell, never exported into the environment, and stay
  out of the page on a self-hosted server.
- `docs/desktop-notifications.md` records what a headless box measured (the
  watcher, dedupe, catch-up and floor tests, the app posting and the page
  receiving its event under Xvfb) and what it could not (a notification or a
  click seen on Windows, macOS or a real Linux desktop, and cost per check on a
  real account).

### Changed

- One `CloudProvider` per process, and a 30 s window over the account device
  list, so one viewer open costs one `msdlistCode` instead of three. The provider
  was built per request — a fresh `reqwest::Client` and a reload of the
  device-token store each time — and `/api/stream`, `/ws/live` and the cold
  start each asked the vendor for the device list again. `/ws/live` and the
  cold start now read the entry the ticket walked, keyed by the vendor session
  and answered per serial.
- The desktop shell hands events to the page through `WebviewWindow::eval` and a
  DOM `CustomEvent`, not Tauri's event channel: the window is loaded from a
  remote origin and a capability only covers the local one, so `event.listen`
  was refused and a click never reached the sheet. The app therefore ships no
  capability file at all.
- The FCM row of `docs/parity.md` is `not needed` for the desktop build: the
  native notification path replaces the vendor push, which stays relevant only to
  a mobile build registered in the vendor Firebase project. The local
  notification row names the desktop process as the main path and the browser
  Notification API as the web fallback.

### Fixed

- A vendor token the cloud revokes is reported as a session that ended instead of
  a cloud failure. The device list answered code `-4004` as a plain rejection
  (`session expired; sign in again`), so the desktop watcher logged it at warn and
  never posted its pause notification, while a route that reads the device list
  answered `502` on the provider path and `400` on the routes that call the client
  themselves. `msdlistCode` now yields `CloudError::Expired`, the variant `refresh`
  already returned, and every route that reads the device list answers `401`:
  `GET /api/devices`, `GET /api/cloud-path`, `GET /api/recordings` and
  `GET /api/stream` where those answered `502`, and
  `GET /api/cloud/device-attributes`, `GET /api/cloud/alarms` and its picture
  route where those answered `400`. A live attempt on `GET /api/stream` now stops
  at that error instead of running the diagnostic walk, so its failure body carries
  no `stage`, `detail`, `elapsed_ms` or `stages`. The watcher reads the check as
  `AlarmReadError::SignedOut` and posts the same single pause notification per
  span. The vendor token is a 30-day JWT, so the path is reached only when the
  cloud revokes it before the hour-long session store expires.
- A desktop alarm check that loses the session no longer goes quiet. The shared
  session expires `PROVIEW_SESSION_TTL` seconds after sign-in (an hour by
  default) and reading it does not renew it, so an app kept in the tray stopped
  checking with one debug line and nothing else. Once a check has succeeded, the
  first later check whose error says the session is gone posts one *Alarm
  notifications paused — open PROVIEW and sign in* notification and logs at warn,
  once per span; the next successful check re-arms it, and a launch that has not
  checked successfully yet announces nothing, because the session store is empty
  at every start. A recovery after two failed checks in a row summarizes the gap
  in one notification instead of replaying the backlog alarm by alarm, while a
  single missed check still toasts; the rule counts checks rather than seconds,
  so timer jitter cannot flip it. The remembered
  window and the device cache now belong to the signed-in account, so an account
  switch drops both and starts the run over, and an empty first pass reports a
  state change so the baseline mark is saved by the watcher rather than only by
  a direct `save()`.
- The desktop **Restart the app** button restarts the app again. The window is
  served from `http://127.0.0.1:<port>/`, an origin Tauri counts as remote, so
  the `restart_app` IPC command was refused and the button did nothing. The page
  now posts to `POST /api/desktop/restart` with the per-run token the desktop
  shell writes into the served page (`window.__PROVIEW_DESKTOP__`), and Rust
  calls `app.restart()`; the command and its IPC registration are gone, and no
  capability was widened to the served origin. The route and the page accept
  only the authority the shell bound (`127.0.0.1:<port>`), so a name that
  re-binds to loopback gets a 403 and a page without the token. A server build
  carries no token, so its view still says to restart the process instead of
  offering a button that cannot work.
- Opening **Settings** while the boot request for the live stream is still
  waiting no longer leaves a player behind. `requestStream` held the player it
  built across the `await /api/stream`, so the `stopAll()` from the Settings
  screen destroyed it and removed it from `state.players`, and the reply still
  called `player.play()`: a `/ws/live` socket opened with nothing left in the
  app able to close it, and the ticket it stored described a camera no screen
  was showing. Every request now builds its own stage player (the one it
  replaces is destroyed and unregistered first), so a reply is dropped when the
  stage no longer owns the player it belongs to; that also covers two quality
  switches whose replies land out of order, where the older reply used to keep
  an unowned `/ws/live` socket alive and, in the other order, overwrite the
  ticket with the quality the user had already left. A `sourceopen` that
  arrives after `destroy()` opens no socket at all. A player replaced before
  its first frame now takes its `loadedmetadata` listener with it, so the
  watchdog it would have started on that event cannot outlive it and report
  `stalled` on a later load that has not shown a frame yet.
  `tests/web/stream-orphan.test.mjs` covers the Settings case, the late
  `sourceopen`, both overlap orders and the replaced player; it fails on
  6c70754 and passes after this change.

## 0.3.4 — 2026-10-28

### Added

- Device settings write with backup and revert. `GET`/`PUT
  /api/device/settings/{encode,network,storage}` write the encoder, network
  and storage blocks over DVRIP, and the `Device settings` page drives them.
  Each write copies the block to `config-backup-<block>-<ms>.json` (0600 in
  a 0700 directory) first, writes, reads back inside a 20 s window it
  carries itself, and restores the copy on a mismatch or a failed write; a
  restore the device does not take is reported as a failed revert instead of
  a clean one. Exercised on the camera over the relay: 10 fps / 1024 kbps
  written and read back with both fields matching, then 12 / 1536 restored
  with FPS, BitRate, GOP and Resolution matching. Network writes stay on
  DNS: address, DHCP, gateway and submask answer 409 because the read-back
  and the revert both travel over that path.

- Alarm message centre. `/api/cloud/alarms` speaks the recovered
  `AlarmCenter` envelope against `access-alc.secu100.net:6602`,
  `/api/cloud/alarms/picture` signs an object through the CssCenter hop, and
  `web/alarms.js` lists time, event and channel, draws the picture, and
  raises one browser notification per new alarm id once the user presses
  Notify me.

- Cloud recordings. `GET /api/recordings?device=&channel=&day=` carries the
  DSS token and the PMS `video_query` call, and the panel maps `StartTime`,
  `StopTime`, `VideoSize`, `IndexFile` and `StorageBucket` from the vendor
  bean. An account without a cloud plan gets the vendor acknowledgement with
  no clips and the page renders its notice.

- Playback. `GET /api/playback/files` lists a device's recordings over the
  cloud tunnel, `GET /api/playback/download` streams one, and
  `web/playback.js` draws the day timeline; a 60 s window was collected as
  84.36 MB in 107 s through the fMP4 splitter, with ffmpeg stopped by
  closing the socket.

- One live tile per device with a layout the browser keeps (`GET /api/grid`,
  `web/grid.js`).

- Fisheye dewarp from the recovered lens model: `/api/fisheye/model`,
  `/api/fisheye/map` and `/api/fisheye/dewarp`, driven by `web/fisheye.js`.

- Talk over the vendor relay when the LAN is out of reach (`GET
  /api/ws/talk`), two audio streams with a configurable buffer.

- AP-mode provisioning: `POST /api/apmode/plan` and `/api/apmode/provision`
  send the DVRIP `NetWork.Wifi` write and the app's own UDP fallback sender
  to a camera that is still on its own access point.

- Device attributes on the account: `GET /api/cloud/device-attributes`,
  `/api/cloud/device-caps` and `/api/cloud/device-groups`, plus rename,
  group, favourite, share and password on the cloud envelope.

- The token login now travels encrypted on the monitor channel, with the
  media bring-up the SDK itself uses.

- Media channel cipher. `crates/core/src/media/dhav.rs` speaks the envelope the
  vendor SDK and the camera use on msgid 1000: AES-128-CBC with a 16-byte zero
  IV, zero padding instead of PKCS#7, base64 in the body, DVRIP header kept.
  The msgid-1000 key is the constant `dashoiahfarqdasr` at `.rodata 0x51b456`,
  which is why the `OPMonitor` claim reply can be read before any exchange.
  Proven on a live session: the captured 384-byte login frame decrypts to its
  374-byte JSON and re-encrypts to the same 384 bytes; the 720-byte claim reply
  decrypts to the capability JSON with the device's RSA public key. The same key
  opens the vendor app's own capture (577-byte login body, 432-byte plaintext),
  and the claim ciphertext is byte-identical between the two. The module carries
  the cipher only, no frame parser: the recording format is a separate
  workstream. `docs/protocol.md` 4C.19 carries the addresses, the body-class
  table for msgid 1412 and the controlled 28-combination sweep with its positive
  control.

- Account lifecycle on the cloud envelope. `POST /api/account/remember` keeps
  the account email and access token in `account.json` (0600) next to the
  settings, and `GET /api/account/remembered` rebuilds the server session from
  that token without the password: measured 401 → `restored: true` → `GET
  /api/devices` 200 on the account's camera. `POST /api/account/signout` drops
  the session and the stored token and records the vendor's raw reply, which is
  `5005 PARAM_FAIL` because the legacy cookie sign-out cannot be authenticated
  from a v3 session (`docs/protocol.md` 10D.3).

- Device bind by serial and by QR. `POST /api/devices/bind`,
  `POST /api/devices/bind/qr` and `POST /api/devices/unbind` drive the vendor's
  `mdadd`/`mdrm` on the same signed envelope the device list uses. The QR codec
  (`XEnCodeStr`/`XDeCodeStr` from libFunSDK) is reproduced byte for byte in
  `crates/core/src/cloud/bind.rs`, with the vendor's own vectors as unit tests,
  and both the bare blob and the `https://d.xmeye.net/CSee?shareInfo=` share
  link parse into a serial. Measured: the real serial answers `4101`, a
  fictional one answers `2000` and appears in `GET /api/devices`, `mdrm`
  removes it, `uuid=no` answers `4104` (`docs/protocol.md` 10D).

- PTZ tours over DVRIP. `GET /api/device/controls/tour` reports the device own
tour lines (`Uart.PTZTour`), the presets it can cycle (`Uart.PTZPreset`) and the
gun/ball locate block, and `POST /api/device/controls/tour/{start,stop,add,clear}`
drives `OPPTZControl` `StartTour`, `StopTour`, `AddTour` and `ClearTour`, which is
the surface the app itself uses (`com.lib.sdk.bean.tour.OPPTZControlBean`).
Measured through the cloud relay: with two presets in the line the lower lens
moved 73.10 and 70.77 mean-luma points between stops while the upper lens stayed
at 0.15-1.07, and `StopTour` dropped it to 0.97-1.57. Every `OPPTZControl` answers
`Ret 100` before it moves, so the listing carries the device configuration and a
separate note on every action says the ack is not motion; start and stop answer
409 when the device holds no such line or the line has no points, and `TourTimes`
is fixed at 1 as the app sends it. `docs/protocol.md` 9A carries the numbers.

- Alarm message centre. `GET /api/cloud/alarms` reads the vendor alarm centre
  (`access-alc.secu100.net:6602`) with the envelope recovered from
  `libFunSDK.so` (`AlarmCenter/Header/*`, `MSG_ALARM_QUERY_REQ` and
  `MSG_ALARM_QUERY_BY_TIME_REQ`), and `web/alarms.js` lists the results with
  time, event and channel, polls on an interval and raises a browser
  notification for alarm ids it has not seen before. Live: 52 alarms for one
  local day, route round-trip 457 ms cold and 77-86 ms warm, a silent host failing in 6 s.

### Changed

- `/api/lens` reads the motorised lens from the device (`Ptz.GunBallLocate`) and
reports `ptzSource: "device"` with a `ptzEvidence` string that quotes the block
and names the host it came from, instead of the measured default it used to
guess. The read never blocks the live stream: the route answers from a cache and
refreshes in the background with one bounded probe (10 s) at a time, keeping a
device answer for 10 minutes and a failure or timeout for 1 minute. While a
refresh runs the last device answer keeps serving, so `/api/lens` never drops to
`measured_default` between refreshes; the measured default stays only as the
fallback when the cache holds nothing at all.
- `lens=top` on `/api/device/controls/ptz` now answers 409 with the code
`ptz_lens_has_no_motor`, the device own locate block in the message, and a
pointer to `lens=bottom`, so the refusal carries evidence instead of a bare
conflict.
- The PTZ capability report names the tour row `ptz patrol` (it was
`ptz patrol over dvrip`, which no client matched) and adds `ptz move` on the
lower lens gated on the device answering (`SupportPTZDirectionControl` or
`Ptz.GunBallLocate`), `ptz move` blocked on the upper lens with the device
reason, and a `ptz locate` row. The pad gained a Tour control that starts and
stops the first tour line that has points and says the ack is not motion.

### Fixed

- The live view toolbar fits phone widths: it wraps to a second row, and at
  640 px and below its buttons show icons only (the label stays the accessible
  name) with the separators hidden. At 360-480 px Talk's label covered Sound,
  and Quality and Full screen sat off-screen.

## 0.3.3 — 2026-10-27

### Added

- `GET /api/cloud-path?device=<serial>` reports what the cloud path can reach
  for one device today: the access server the status cluster names, the media
  agent the RPS answers with, and whether the relay accepted the handshake. It
  rides the same session cookie as `/api/stream`, so the browser can call it
  without a second sign-in. The live screen shows its own error immediately and
  appends that line when it arrives, so "not wired" no longer hides whether the
  relay is reachable. It also makes the relay transport part of the shipped
  binary, which the 0.3.2 binaries did not carry.

## 0.3.2 — 2026-10-27

### Added

- Cloud live path, in the crate but not in the shipped binary. The vendor
  relay is implemented and exercised by `examples/rps_probe`: the status cluster
  names the access server, an `AgentProtocol` request buys a media agent for the
  session, the base64 handshake is answered with `ErrorNum 200`, and DVRIP rides
  that tunnel up to the `OPMonitor` claim, measured from a machine with no route
  to the camera. Nothing calls it yet: `CloudProvider::stream()` still answers
  `NotImplemented`, so the linker drops it and `strings` finds no trace of it in
  the 0.3.2 binaries. It ships when the encrypted media login does;
  `docs/protocol.md` 4C.6-4C.12 records exactly what is proven and what is not.

- Cloud device status. `GET /api/devices` now reads `pub-status.secu100.net`
  `/v1/status/third` across all seven clusters and answers `online`,
  `online_status`, `online_checked_at` and `online_detail` instead of a
  hard-coded false. The live screen blocks only on a known offline status and
  prints the vendor's reason when the status cannot be read.

### Changed

- `GET /api/devices` asks the status clusters in parallel and stops once one of
  them reports the access server, so the device list answers in 0.11-0.18 s
  where it took 0.79-4.09 s when every port had to time out first.
- The Windows release jobs install Strawberry Perl and NASM and pin
  `OPENSSL_SRC_PERL`, because the vendored OpenSSL build cannot use the perl
  that the bash shell puts first on those runners.

## 0.3.1 — 2026-10-27

### Changed

- The cloud app identity ships with the app. `release` bakes
  `PROVIEW_APP_UUID`, `PROVIEW_APP_KEY` and `PROVIEW_APP_SECRET` from
  repository secrets into the binary at build time and fails the build when one of
  them is missing, so signing in asks for an email and a password and nothing
  else. The environment still wins per value when it is set, which is how a build
  made from source brings its own identity; without one the notice says the build
  was made without a cloud identity and points at the documentation, instead of
  asking a user to set variables. The three fields moved from Settings →
  Configuration → Cloud to Settings → Self-built builds. Rotating the identity now
  means a new release: a binary keeps the identity it was built with.

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
