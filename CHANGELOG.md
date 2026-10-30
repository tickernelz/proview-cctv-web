# Changelog

Every release is a tag on `main`; each one publishes 8 server binaries, the desktop
installers and a `SHA256SUMS` file from the same workflow. Format follows
Keep a Changelog; this project uses `0.MINOR.PATCH` and does not promise API
stability before 1.0.

## Unreleased

### Added

- PTZ at parity with the phone app's live and preset screens, over the cloud
  relay: the pad keys, arrow keys and zoom/focus buttons now move while held and
  stop on release (`/api/device/controls/ptz/press` and `/ptz/release` keep one
  DVRIP session per hold; a tap keeps moving 800 ms like the app); a click on the
  top lens picture aims the moving lens there (`/ptz/locate`, `OPPtzLocate`);
  "Edit presets" in the pad saves the current position as a preset, renames and
  deletes presets and edits the tour stops (`/preset/add`, `/preset/rename`,
  `/preset/delete`, `/tour/remove`). Zoom, focus, calibration
  (`/ptz/calibrate`) and the panoramic PTZ range (`/panorama`) follow the
  device's ability flags and stay hidden on this camera, which reports none of
  them; the capability table shows why. Every one of these writes reads the
  device's ability on its session first and answers 501 with the device's
  wording when the flag is missing, before anything is sent. A hold stops by
  itself after `PROVIEW_PTZ_HOLD_MS` (30 s), and leaving the page mid-hold
  sends the release as a beacon.
- Device settings work through the vendor cloud: `/api/device/settings/{encode,network,storage}`
  and `/api/device/controls/settings` take `device` and open DVRIP over the relay,
  so they answer without the camera on the local network. New blocks with the
  mobile app's gating: SD card usage by partition kind and the missing-card alarm
  (`Storage.StorageNotExist`), the record schedule (full-time or alarm-only),
  clip length, record stream (`Storage.RecordStream`), lens splicing, record
  audio (`/api/device/settings/record`), smart encode behind `Encode264ability`,
  an encoder form bounded by `EncodeCapability`, the burned-in channel title and
  time overlay (`/api/device/settings/osd`, `AVEnc.VideoWidget` + `ChannelTitle`,
  optional `TitleDot` bitmap), and Format SD (`OPStorageManager`, cmd 1460)
  behind a typed `FORMAT` confirmation.
- Pan and tilt work through the vendor cloud (ZHA-4281): `/api/device/controls`,
  `/api/device/controls/ptz`, `/api/device/controls/presets`,
  `/api/device/controls/preset/goto` and `/api/device/controls/snapshot` open the
  DVRIP session over the same RPS relay the live stream uses, so none of them
  needs the camera on the local network. The capability report in cloud mode is
  read from the device (`Ptz.GunBallLocate` binds the motor to the lower lens,
  `Uart.PTZPreset` lists the presets) instead of answering "unconfigured"; the
  upper lens keeps its own refusal, and `PROVIEW_LENS_PTZ` stays an override
  rather than the only source.
- `tools/acceptance/cloud_ptz.py`: one command that proves the claim with pixels.
  It refuses to run while the camera answers on the LAN, parks the lens at a
  preset, moves it, and measures the shift on the busiest tiles of each lens
  crop by phase correlation against the noise floor between two frames taken
  before the move.
- The control tunnel uses the account device token over the relay, the same
  credential live and playback use (PR #101): `open_control` logs in through
  `device_login` without claiming the monitor, and every control request and
  its reply are sealed with the 16-character session key the way playback does
  it. No camera password is read from the environment on this path.
- Two-way audio reaches a cloud camera without its password (ZHA-4292):
  `/ws/talk` opens the vendor relay with `CloudProvider::open_talk`, whose DVRIP
  session logs in with the account device token, and the capability report in
  cloud mode now answers `two-way audio: supported` over the relay instead of
  asking for `PROVIEW_DEVICE_PASS`. The camera password stays the direct-address
  path only. The session-key seal of the control channel is not applied to the talk
  claim: sealed `1434` is refused with `Ret 117`, plain `1434` is answered, and plain
  `1430`/`1432` are accepted (sealed `1430`/`1432` were not sent, so they are
  untested - 4C.29).
- `tools/acceptance/talk_acceptance.py`: one command that proves the talk path
  end to end. It refuses a camera that answers on the LAN, refuses to start the
  server with a camera credential in its environment, opens `/ws/talk` with
  silent PCM, and fails unless the capability row is supported, the report has
  frames and bytes out, and the camera sent audio back.

### Changed

- The Move button no longer greys out without a reason: while it is blocked the
  lens menu names the device's own refusal and the button carries the same text
  as its description, and the control panel asks the server for the capability
  report of the camera on screen (`device`), which is what lets a cloud camera
  report its motor at all.

### Fixed

- Live view no longer opens an ffmpeg console window on Windows (ZHA-4280):
  every ffmpeg spawn now carries `CREATE_NO_WINDOW`, so the desktop shell shows
  only its own window.
- H.264 cameras no longer start ffmpeg for the copy remux. The live path
  depacketizes the RTP stream itself (single NAL, STAP-A and FU-A) and muxes
  fMP4 in process, so ffmpeg runs only when a stream must be transcoded, such as
  HEVC turned into H.264 for a browser without HEVC support.
- The hub now replays the fragments since the last keyframe instead of a single
  fragment, and transcode fragments carry up to 10 access units with their real
  keyframe flag. A viewer that joins late receives the whole current GOP, and one
  that falls behind resumes on the next keyframe-start fragment with no gap.

## 0.3.7 — 2026-10-29

### Security

- The settings route answers only the machine it serves, and only a page it served
  (ZHA-4132). `GET` and `POST /api/settings` carry the camera user and password, the
  cloud address and the cookie secret, and answered any request that reached the port,
  so a name that resolved to this machine through DNS rebinding, or a request from the
  network, could read and rewrite them. The desktop build now answers only the
  authority its shell bound (`127.0.0.1:<port>`) and only to a request holding a live
  sign-in or the per-run page token the shell writes into the page; anything else is
  `403` for another `Host` and `401` without a credential. The standalone server answers
  only a request that arrived from this machine (a loopback TCP peer; over a direct
  connection a client on the network cannot forge it, though a forwarder on this
  machine gives every client it carries that peer) on a loopback `Host`; every other
  peer, `Host`, or an `Origin` that is not this server's own, is `403`. The sign-in screen keeps its
  settings on this machine, so a wrong camera address can still be fixed before
  signing in.
- AP-mode provisioning is pinned to the camera's own network (ZHA-4026). The route has
  read no network target from the body since PR #20; it dials two constants, and those
  two are now held to an allowlist by a test — private (`10/8`, `172.16/12`,
  `192.168/16`), link-local or limited broadcast, never a public, loopback,
  unspecified or multicast address — so a later edit that moves either constant out of
  the camera's network fails the build. The TCP login uses `admin` with only the
  password typed in the form, never `PROVIEW_DEVICE_USER` / `PROVIEW_DEVICE_PASS`,
  which stay with the host the app was configured for.

### Changed

- The tray says why the alarms are quiet (ZHA-4154). Its first line now reads
  `Alarm notifications on · session ends 21:14` while the shared session is live and
  `Alarm notifications paused · session ended 21:14` once the session has gone, in
  local time, so the hour is visible without opening the window — and an explicit sign
  out records the moment it happened rather than the hour the session would have
  expired. Off reads
  `Alarm notifications off` and a run that never had a session says it is waiting for
  one. The pause notification still posts once per loss, and the session lifetime
  itself is unchanged: the store stays the only source of truth and no vendor token
  is kept anywhere else.

## 0.3.6 — 2026-10-29

### Changed

- The page has a refined look (ZHA-4181): one icon family (Lucide, built into
  the inline sprite, so no icon runtime ships), a blue accent for what is
  pressed, selected, checked or primary, no text under 12 px, tabular numbers on
  clocks and timestamps, a 4 px spacing grid and 160/200 ms motion that
  `prefers-reduced-motion` still switches off. The alarm sheet, playback and
  cloud recordings follow the light theme now: they used to inject their own
  dark colours. The alarm bell is an icon instead of an emoji. Every token pair
  passes AA (`tools/contrast.py`, 76 pairs). Design notes in `DESIGN.md`, the
  options considered in `docs/design/redesign-v2/`, the library decision in
  ADR 0002.
- Every screen fits from 320 to 1920 px wide: sign-in becomes one column below
  900 px, settings stacks below 768/640 px, the top bar tightens below 480 px,
  and "every camera" uses 1, 2 or 4 columns by width instead of 3 everywhere.
  Buttons in the top bar and the control row are 44 px on touch screens.
- Live's caption, control row and lens and quality menus are Vue now (ZHA-4181):
  one component over a reactive store, menus from Reka UI 2.11.0 placed by
  floating-ui so they stay inside the window at every width. Ids, test ids,
  keyboard shortcuts and the look are unchanged (screenshot baselines pass
  as they were). Keys pressed in an open menu stay in the menu: Escape closes
  the menu without closing the pan and tilt pad or the device sheet, and arrows
  move through the menu instead of panning the camera. The page JS grows by
  67 KB gzip for the Vue runtime and Reka UI, inside the 80 KB budget; later Vue
  screens reuse it.
- Cloud recordings is a Vue panel (ZHA-4181): it opens on the camera on screen
  instead of the first in the list, puts focus in Day and hands it back to Cloud
  on Escape or Close (Escape did nothing before), shows "Loading…" on the button
  and ignores a second press instead of sending the same request twice, names
  the camera and day when a day is empty, shows a failed load in red with the
  warning icon instead of grey text, and returns to sign-in when the session
  ended. The title reads "Cloud recordings".
- `apps/web/src/legacy/` is gone (ZHA-4181): the add-a-camera panel on the
  empty Live screen is Vue (`src/live/AddCamera.vue`). It is shown open with a
  heading instead of hidden behind a disclosure triangle; an empty serial or
  code is caught before any request and names what to type; the button says
  "Adding…" and ignores a second press while the bind runs; a refused bind is
  a red alert with the warning icon and the cloud's reason; a bound camera
  opens in Live at once, and a serial or camera password typed there is
  cleared on sign-out instead of being sent for the next account. Sign out
  forgets the remembered account again: since Settings moved to Vue the old
  panel script's listener on the Settings button was never attached, so a
  reload after sign-out restored the session without a password. Sign out now
  asks the server to forget the stored token first and ends the session after
  that answer, so the two requests no longer race for the same session. The
  switcher no longer shows a green status dot when there is no camera.
- `apps/web/src/legacy/app.js` is gone (ZHA-4181): the lens picker, lens
  crop, pan-and-tilt pad, presets, tour and the device-control gates moved to
  `src/app/lens.ts` and `src/app/controls.ts`, and the pad is rendered by
  `LiveMain.vue` from the store. The pad keys, Tour and the presets behave as
  before.
- The Live picture area is Vue (ZHA-4181): a failed stream is announced as an
  alert with the warning icon in its title; a 502 without stage detail says
  its reason is the line below instead of pointing at a tooltip nobody can
  hover on a touch screen; an offline camera says it comes back once that
  camera answers and the list is refreshed, without repeating "cloud status
  offline" from the banner above it; the error actions wrap on a narrow
  screen. The stream, view switching, snapshot, recording, full screen and
  the Live keyboard shortcuts moved from `legacy/app.js` to `src/app/*.ts`.
- The Live top bar, camera menu, banner and keyboard list are Vue (ZHA-4181):
  an offline camera banner no longer repeats "cloud status offline" after
  saying the camera is offline, and its button says what it does: "Refresh
  the list" when it reloads the cameras, "Retry now" only when it retries the
  stream after the network came back, as an outlined button with the refresh
  icon instead of an underlined link; a camera list that fails to load uses
  the warning icon; the keyboard list gives focus back to where it was when
  closed from its button; the camera menu keeps digits 1-9 as camera
  shortcuts while it is open; the address of another screen closes the
  Playback overlay.
- Settings is Vue (ZHA-4181): a save the server refuses shows its reason as a
  red alert in the save bar and keeps focus on Save (it used to land only in a
  grey line at the top of Configuration while the bar still said "Unsaved
  changes"); leaving with unsaved edits is an alert that says "Save or discard
  1 changed field before leaving." instead of "field(s)"; a field that needs a
  restart carries a Restart tag next to its label instead of a grey sentence,
  and a changed field keeps an accent outline until it is saved; Run checks
  shows "Checking…" and keeps focus, and each check is a sentence with a check
  or warning icon and the HTTP status after the reason instead of a monospace
  "400 …" line; a provider switch the server refuses stays on screen as an
  alert and the radio returns to the provider that still answers (the error
  used to be overwritten at once); the capability table uses check and cross
  icons in its pills, and the white light button says why it is disabled.
- Sign-in is Vue (ZHA-4181): the cursor starts in Email; a wrong password is
  an alert with the warning icon in our own words, and the password stays
  selected; Sign in shows "Signing in…" while it works and a second press sends
  nothing; "This network" on a cloud server says to use Account instead of
  showing a disabled form; a failed camera probe is a red alert, not grey text;
  Remember is a switch next to the form instead of an injected checkbox; Open
  Settings is an outlined button instead of an underlined link.
- Every camera (the grid) is Vue (ZHA-4181): on a phone the tiles no longer
  overlap (a tile's HD/SD and caption sat on the next tile's picture) and a
  saved 2- or 3-column layout falls back to one column below 641 px; a tile
  that is connecting says so, a failed tile centres its reason with a Try
  again button instead of a left-aligned underlined link, an offline tile says
  "offline" once instead of twice; HD/SD is an outlined button; a layout the
  server refuses to save is a red alert with the warning icon; the count line
  drops the repeated "layout auto".
- Playback is a Vue screen (ZHA-4181): it opens on the camera on screen
  instead of the first in the list, focus moves to it and Escape returns to
  Live (Escape did nothing, and number keys switched the camera behind it, also
  after a click on the strip); `#playback` opens it again (the link fell back
  to #live); the
  summary names the camera and day; a failed file query is a red alert instead
  of "No files in that range." with the reason in grey under the video;
  switching day or camera while a list loads no longer lets the older answer
  win; the strip has hours below it, a legend, and the playing clip outlined;
  motion rows say "motion" instead of only being greyer; the day arrows are
  icons instead of ‹ ›; Play from here and Download are disabled until a clip
  is chosen; the slider shows its time; an expired session returns to sign-in.
- Devices is a Vue sheet (ZHA-4181): the result of Rename, Save, Share,
  Accept or a password change shows in a line pinned to the bottom of the sheet
  instead of under a form taller than the window, with a red warning for a
  refusal; a working button says "Renaming…" and ignores a second press; the
  three jobs are on tabs (Name, Sharing, Password) instead of one long form;
  cameras are a radio group with arrow keys, focus moves into the sheet and
  Escape or Close hands it back to Devices; a list that fails to load is an
  alert with Try again instead of grey text above an unusable form, and Accept
  stays available there and on an account without cameras; Favourite is a
  switch; the sheet sits in the shared corner below the top bar instead of
  filling the window and covering Settings; it closes on Settings and sign-in,
  and signing out clears the typed account password, which stayed in the page
  before; Indonesian placeholder names are English like the rest of the page.
- Device settings is Vue (ZHA-4212, `src/live/DeviceSettings.vue`): the
  result of a write (kept, reverted, or a failed revert) stays in the status
  line after the read-back reload instead of being replaced at once by "Read
  from the device just now.", so a 409 revert is still on screen; a save button
  reads "Saving main…" with `aria-busy` and ignores a second press until the
  read-back is in; every field label is tied to its input; the page reloads
  whenever it is shown, including through the keyboard and `#settings/device`,
  and a 401 returns to sign-in. Ids, fields, the writes sent and the
  DNS-only network rule are unchanged.
- Alarm messages is a Vue sheet (ZHA-4181): the bell counts alarms right after
  sign-in instead of after the first 20 s poll or the first open, opening it
  puts focus on Camera and Escape or Close hands it back to the bell (Escape
  did nothing before), and it replaces Cloud recordings in the shared corner
  instead of drawing on top of it. Changing camera while a check is out no
  longer shows the previous camera's alarms under the new one. Empty and
  loading states name the camera and the window, a failed check is red with
  the warning icon instead of grey footer text, the footer reads "3 alarms ·
  updated 22:16" (timings moved to its tooltip), rows show the channel only on
  multi-channel cameras and count it from 1 like the caption, and a row
  without a picture shows an icon instead of an empty box.
- The fisheye overlay and Talk are Vue-side modules now (ZHA-4213):
  `FisheyeOverlay.vue` puts the WebGL2 dewarp canvas over the live video for
  `?fisheye=equirect|ptz` (same shader, same `window.ProviewFisheye` and
  `window.ProviewFisheyeLive` handles), and `live/talk.ts` carries the 8 kHz
  AudioWorklet uplink unchanged. The overlay now takes its lens from the video
  once the video has a size, so a 512 px wide stream no longer keeps a zero
  radius. e2e specs check the shader against `/api/fisheye/map` and Talk
  against a fake peer.

### Added

- An end-to-end suite (`e2e/`, Playwright 1.64.1) against the served build with
  the fixture provider: sign-in, devices, live, lenses, PTZ and presets, record,
  white light and flip, settings, alarms and the grid, in Chromium and WebKit,
  plus a viewport gate from 320 to 1920 px, touch-screen projects and screenshot
  baselines in both themes. CI runs it as the `e2e` job.

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

- The page build compiles Vue single-file components (`@vitejs/plugin-vue`,
  type-checked by `vue-tsc`) and Tailwind CSS v4 (`@tailwindcss/vite`, theme and
  utilities only, no Preflight reset), ready for the Vue screens. The page floor
  rises to Tailwind's: Chrome/Edge 111, Firefox 128, Safari/WKWebView 16.4, and
  the macOS desktop app now requires macOS 13.3 (`docs/webview-matrix.md`).
  Nothing visible changes yet.
- The page scripts are ES modules bundled by Vite into one hashed
  `/assets/index-<hash>.js` (gzip -9 57,391 B in ten files -> 40,396 B in one).
  `/app.js`, `/grid.js` and the other per-script URLs answer 404. The talk
  script's `startTalk`/`stopTalk` used to replace the older copies in `app.js`
  through the shared global scope; those dead copies are removed and `app.js`
  imports the talk module's.
- The page moved from `web/` to a Vite app in `apps/web`. `npm run build`
  emits `apps/web/dist/` with a hashed stylesheet, and the server embeds that
  directory with `rust-embed` instead of one route per file, so build the page
  before `cargo` (see README). The scripts are served unchanged; `/tokens.css`
  and `/style.css` became one `/assets/index-<hash>.css`. CI and release jobs
  run `npm ci --include=dev && npm run build` with an npm cache, and `cargo tauri build` does
  the same through `beforeBuildCommand`. `PROVIEW_WEB_DEV=1 cargo tauri dev`
  opens the desktop window on the Vite dev server for hot reload.
- One `CloudProvider` per process, and a 30 s window over the account device
  list, so one viewer open costs one `msdlistCode` instead of three. The provider
  was built per request — a fresh `reqwest::Client` and a reload of the
  device-token store each time — and `/api/stream`, `/ws/live` and the cold
  start each asked the vendor for the device list again. `/ws/live` and the
  cold start now read the entry the ticket walked, keyed by the vendor session
  and answered per serial. A serial the window does not carry is asked again
  instead of refused, and a fresh `/api/devices` fetch replaces the window, so a
  camera bound or unbound inside those 30 s still answers correctly. The vendor
  identity stays per request: `PROVIEW_APP_UUID/KEY/SECRET` saved in Settings
  after start reach the next call, as they did before the provider was shared.
- Parallel cold opens in one session share a single device list, including when
  the list lacks the camera and when the cloud fails. The window was read before
  the fetch, so N tiles of a grid opening at once could each miss it and each ask
  `msdlistCode`; a `tokio::sync` mutex per vendor session now gates the fetch, and
  a waiter adopts whatever the winner produced when that finished after the
  waiter started. That is one call per burst for hits, for serials the account
  does not list, and for a failed or timed-out list — the failure is handed to
  every waiter instead of being retried one at a time, so a grid against a dead
  cloud no longer waits N times the 20 s request timeout. A sequential open still
  asks again, and an explicit refresh (`/api/devices`) never adopts: it waits its
  turn and then asks, so a camera bound or a nickname changed while another fetch
  was running is still in the list it returns. The cost of adoption is that a
  camera bound inside one round trip is not seen by the opens already waiting,
  which the next open or `/api/devices` picks up. A gate is dropped once no
  caller holds it, so the map cannot grow with every session that opens a camera.
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

- A camera that refuses the device token is no longer offered one. The `AdminToken`
  a password login returns was stored for every serial, so a camera that answers
  `Ret` 117 (or 137, as the live unit here does) to the token login paid the token
  attempt plus a second tunnel on every open, forever: nothing could make that token
  work, and the token the fallback stored in its place was refused the same way.
  `CloudProvider` now records the refusal against the serial — 117 and 137 are the
  two answers that mean this device will not take tokens — stops offering a token to
  that serial for the life of the process, and no longer stores the `AdminToken` a
  password login hands back for it, so the next open goes straight to the password
  login on its first tunnel. A device that accepts the token keeps the fast path, any
  other non-100 answer still drops the token and tries again on the next open, and an
  accepted token clears the mark.
- The password fallback after a stale device token opens its own tunnel instead of
  reusing the socket the token login left behind. A token reply and a password
  reply carry the same message id (`1001`), so a token reply that arrived after the
  token loop gave up — or after the six 80 ms drains that used to run before the
  fallback — was read as the password reply: the session belonged to the old login,
  `AdminToken` was never issued, and the stream ran on a session the password never
  owned. The tunnel setup in `CloudProvider::open_media` (status query,
  `access_server`, `agent_request`, `assign`, `handshake_frame`, `tunnel::open`,
  `accepted`) is split in two: a relay lookup (status query, access server) and
  a dial (assign, handshake, accepted). The fallback only dials again, so a stale
  token no longer repeats the pub-status fan-out whose answer it already has, and
  `drain_late_frames` is gone. Measured on the live relay: a second tunnel in one
  viewer open is accepted with the same `session_id` (`ErrorNum 200`), the first
  tunnel stays up and keeps its claim; a stale token leaves one pub-status fan-out
  per open and costs one dial, `assign` 73–98 ms plus handshake 22–45 ms. The
  `open_media` wall on the device, three samples each, was 564–701 ms with no
  stored token and 653–986 ms with a stale one, against the drain's 80 ms floor
  and 480 ms worst case; this device refuses stored `AdminToken`s (`Ret` 137), so
  the accepted-token arm has no live sample. Against a peer that answers the token login
  700 ms late — later than the whole drain — the pre-fix code returns a session
  owned by the late reply and stores its `AdminToken`; the fixed code answers the
  password login on the second connection.
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
  checked successfully yet announces nothing from the check, because the session
  store is empty at every start. A recovery after two failed checks in a row
  summarizes the gap in one notification instead of replaying the backlog alarm
  by alarm, while a single missed check still toasts; the rule counts checks
  rather than seconds, so timer jitter cannot flip it. The remembered window and
  the device cache now belong to the signed-in account, so an account switch
  drops both and starts the run over, and an empty first pass reports a state
  change so the baseline mark is saved by the watcher rather than only by a
  direct `save()`.
- A window with no session and no successful check now says so when it goes to
  the tray. With alarm notifications on, the first close or minimize to the tray
  posts the same *Alarm notifications paused — open PROVIEW and sign in*
  notification the failing check posts, and the rest of that run stays quiet;
  the still-running hint waits for a later hide instead of stacking a second
  notification. It reads the live session store, so a page that has signed in
  stays silent even before its first check, and a run that has checked
  successfully leaves the pause to the check path. The run now lives in the
  desktop shell, so the window event reads the same `last_ok` the check keeps
  instead of a copy.
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
- A cloud live setup that outlasts a reply deadline no longer loses the bytes it
  had already read. `Connection::recv` read the 20-byte header and then the body
  with two separate exact reads, so a timeout that fired between them threw away
  the header bytes it had consumed and the next frame began in the middle of the
  one before it; the pre-claim reply wait, the token-to-password login fallback
  and the monitor claim reply wait all continued on that connection. The reader
  now keeps what arrived in a buffer that survives a cancelled read and hands a
  frame out only once its header and declared body have both arrived, which also
  covers the settings and control reads that share the type.
- Cloud live video logs in as the phone app does, so it no longer needs the
  camera's user and password. `DEVAPI::DecLoginToken` (0xbd0858) opens the
  44-character account token with the device serial as the AES-128 key, and that
  16-character plaintext is the `TOKEN` login name; the old code offered the
  44-character value raw and the camera answered `Ret 137` every time
  (`docs/protocol.md` 4C.27). A device the token store does not know now uses the
  token from the account's own device list, and a build with the opening removed
  fails the acceptance run at the live stage while the shipped one passes 300 s
  with no `PROVIEW_DEVICE_USER`/`PROVIEW_DEVICE_PASS` set. The decoder keeps the
  native guard instead of a stricter one: a plaintext of 1 to 63 bytes is accepted
  and no printable filter runs, so a name holding a byte outside the printable
  range still reaches the camera, and a stored value the decoder cannot open is
  offered exactly as the store holds it.
- Recorded playback no longer stops after the primed frames when the device
  pauses mid-frame. `playback::Session::recv` read the header and the body with
  the same two exact reads, and the clip route stops priming on a 3 s read
  deadline and then keeps that session for the rest of the window; a frame cut
  short by the deadline left its body to be read as the next header, decode
  failed and the pump ended. The session now keeps arrived bytes in a buffer
  that outlives a cancelled read, so the pump picks up the interrupted frame
  where the prime loop stopped. Two tests stall past an 80 ms deadline in the
  middle of a header and of a body before finishing the frame: both fail with
  `early eof` on the old reader and pass on the new one. The buffer starts at
  256 KiB rather than empty, because a buffer that grows mid-stream moves the
  half frame it already holds on every refill: on a loopback flood the read
  median falls from 20.4 to 5.3 us per 32 KiB frame and from 7.4 to 1.1 us per
  8 KiB frame (release, 21 samples, 256 frames each).

## 0.3.5 — 2026-10-28

### Added

- Cloud live in the player. `GET /api/stream` answers a websocket ticket and the
  route starts one cloud source per stream key: the provider opens the relay
  tunnel, logs in to the device, waits for the three HEVC parameter sets and
  feeds the same ffmpeg and fMP4 publisher the RTSP path uses, so the view needs
  no change. `docs/protocol.md` 4C.24 and 4C.25 carry the sequence and the
  measurements (first frame p50 1835 ms, p95 3846 ms over ten cold starts; a
  300 s stream at 0.87 s of server CPU), and 4C.26 the cost per stream.
- A device token store per serial: AES-256-GCM with a 0600 key file, the serial
  bound as associated data, `forget` to revoke, and writes serialised across the
  process.
- `PROVIEW_DEVICE_SERIAL` in Settings under Local network. The camera user and
  password are only sent to that serial, and a cloud ticket now answers 501 with
  `code: "misconfigured"` naming the setting when the device could never log in,
  instead of failing later with a generic message.

### Fixed

- The live start body went out under msgid 1413 (the claim) instead of 1410
  (`0x0582`), which is why every earlier cloud run counted zero frames.
- The cloud live socket is scoped to the account that owns the serial: a session
  asking for another account's camera answers 404 before it subscribes.

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
