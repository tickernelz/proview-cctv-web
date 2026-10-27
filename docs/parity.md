# Feature parity matrix

Every row is derived from an artifact of the shipped APK (`com.proview.cctv`
1.0.4 / versionCode 10403), not from a guess. Evidence names the manifest
component, resource or native symbol it came from. Status values:

- `done` — implemented and exercised in this repository.
- `partial` — implemented but missing at least one acceptance property.
- `blocked` — cannot be built from what is known; the missing fact is named.

No row is silently dropped: a feature that the APK has and this project does
not is listed as `blocked` with the missing input.

## Platform core

| Feature | Evidence | Status | Note |
|---|---|---|---|
| Cloud account login | `com.xworld.activity.login.*`, `IXMAccount::AccountLogin`, `XM_Video_Login_XMAccount` | done | the signed v3 envelope is implemented in `crates/core/src/cloud/envelope.rs`: email + password → `accessToken`, the platform exchange and the signed `/v3/<tsid>/<sig>.rs` request, exercised by `crates/core/tests/envelope.rs` and by `POST /api/login`, with no vendor binary at run time. The v2 portal envelope and password digest remain unsolved for our own client. See `protocol.md` 3A and 3B |
| Session kept server side | design | done | private `pvw_session` cookie → in-memory vendor session; vendor token never reaches the browser |
| Session expiry handling | design | done | server expires and returns 401; any 401 on a signed-in call returns to sign-in with what happened and when, and "Sign in to resume" reopens the same camera |
| Remember account | `SplashActivity`, `loginExist`, `savePwdToken` strings | done | `POST /api/account/remember` writes `account.json` (0600, email + access token + `exp` + a per-browser secret) and sets `pvw_remember` (HttpOnly, SameSite=Strict, 30 days); `GET /api/account/remembered` rebuilds the server session from that token with no password only for the browser holding that cookie, and refills the sign-in email. Measured 2026-10-27: `GET /api/devices` 401 with no cookie, `remembered` answered `restored: true`, then `GET /api/devices` 200 with the account's camera; `POST /api/account/signout` clears the token and the same call then answers `restored: false` |
| Logout | `IXMAccount::AccountUnBindingWX`, `/webservice/app/user/logOut` | partial | local sign-out is complete: `POST /api/account/signout` drops the calling session, every other session carrying the same token, the stored token and the `pvw_remember` cookie, and keeps the email; a request with no session and no remember cookie cannot clear the owner's token. The vendor side has nothing to revoke here: `XM_Video_Logout_XMAccount` sends `/webservice/app/user/logOut` with the cookie of the legacy portal session we never open, `rs.xmeye.net` answers `5005 PARAM_FAIL` for it exactly as it does for the control path `/zzz`, and the v3 account service has no sign-out route (`logOut`, `logout`, `userLogout`, `signOut`, `quit`, `userexit`, `zzzNoSuchMethod` all answer the same Spring 404 with the path echoed). `AccountUnBindingWX` is a WeChat unlink (`userunbindWx`), not a sign-out |
| Device list | `DeviceFragment`, `com.xworld.data.GroupDeviceListResp`, `IXMAccount::GetDevList` | done | `GET /api/devices` calls `msdlistCode` through the signed envelope and returns the account's devices (one device on the test account); the shared-device list on `as-ams.jftechws.com/mdsharemylist/v1/<tsid>/<sig>.rs` is still unimplemented. See `cloud-app-surface.md` and `protocol.md` 3A |
| Device online status | `pub-status.secu100.net:7700-7706`, `IXMCloud::GetDevsStatus` | done | mapped and live: `POST http://pub-status.secu100.net:<port>/v1/status/third` with `StatusProtocol/MSG_STATUS_MULTIQUERY_REQ` answers per-device `Status`, `WanIP`, `ServerIP:ServerPort`. The ports are separate clusters, so the client asks all seven and reports online when any says `Online`; measured 2026-10-27 09:35 UTC, `7f92970ddf5282743hrc` online on 7703 and 7705, offline on the other five, `WanIP 103.85.122.66` matching the caps `lastWanIp`. `GET /api/devices` carries `online`, `online_status`, `online_checked_at` and `online_detail`. See `protocol.md` 4A |
| Device identity and capabilities by serial | iOS capture 2026-10-27: `POST https://caps.xmcsrv.net/api/asMsgQ` `{"method":"reqCaps","body":{"caps":[…],"sn":…}}` with no session | done | `GET /api/cloud/device-caps?sn=<serial>` over `cloud::app::CapsClient`: model, hardware id, firmware, `lastWanIp` and the capability block; empty serial 400, unknown serial 404. See `cloud-app-surface.md` |

## Application shell

| Feature | Evidence | Status | Note |
|---|---|---|---|
| Self-hosted HTTP server | `crates/server`, `--listen` | done | 8 target binaries in every tag release, `v0.3.3` included (linux gnu/musl x86_64+aarch64, windows msvc x86_64+aarch64, macOS x86_64+aarch64) |
| Native desktop shell (Tauri 2) | `src-tauri/` | done | CI builds it on ubuntu-22.04, windows-latest, macos-latest; each tag release carries msi/nsis, dmg (x64/arm64/universal), AppImage/deb/rpm |
| Installer signing | — | blocked | no Apple Developer or Windows signing certificate (decision 2026-10-26); artefacts ship unsigned with `SHA256SUMS` |
| Desktop app run on a real desktop | — | partial | Linux: launched under Xvfb on WebKitGTK 2.54 for v0.3.0, signed in with keyboard and pointer input, UI rendered in both themes; live H.264 needs the GStreamer H.264 decoder there (`webview-matrix.md`). Windows and macOS windows not launched yet |
| Vendor-credential storage in OS keychain | requirement | partial | `src-tauri/src/keychain.rs` stores one JSON blob per profile through `keyring` (macOS Keychain, Windows Credential Manager, Secret Service), read at startup and writable with `proview-desktop --store-credentials`; the CLI round trip is exercised, the desktop window is not |
| Configuration in the app | requirement | done | `settings.json` in the data directory, `GET/POST /api/settings`, and a Settings view listing all twenty keys with labels, hints and a restart marker; the view opens from the sign-in screen before any login; successive saves merge; secrets are never returned. See `settings.md` |
| Provider seam (`cloud` / `local`) | requirement | done | `crates/core/src/provider/`: one `Provider` trait for device list, live, playback, alarms and settings, with `CloudProvider` (default) and `LocalProvider` behind `Router` |
| Mode selection and fallback | requirement | done | `GET/POST /api/config/mode`; desktop persists the choice in `app_data_dir/mode.json`, server keeps it per session; local without a host or with an unreachable host falls back to cloud and reports why |

## Live view

| Feature | Evidence | Status | Note |
|---|---|---|---|
| Device reachability probe | `CNetDevice`, DVRIP login | done | `POST /api/device/probe` logs in to the IPC at `192.168.0.100:34567` with the vendor "sofia" 8-character hash: `Ret: 100`, `SessionID 0x00000017`, `DeviceType "IPC "`, `ChannelNum 1`. An empty or 32-hex hash is answered `Ret: 203`, which is where the old `ret 203` report came from |
| Recorded-clip playback path | `Fun_MediaDownloadM3u8ByUrl`, `CMediaFile` | partial | Annex-B clip → fMP4 is served and plays in Chromium (verified with ffprobe + in-browser decode); the live transport feeding it does not exist yet |
| Local RTSP live view | RTSP client inside `libFunSDK.so` | done | `crates/core/src/rtsp.rs` (Digest session, SETUP/PLAY over TCP interleaved) plus `media/rtp.rs` and `media/hevc.rs` pull the IPC at `192.168.0.100:554`: 72 access units in 6 s, all decodable by ffmpeg, played in Chromium through the fMP4/WebSocket hub |
| Live video over cloud P2P | `DeviceMediaActivity`, `CMediaChannel`, `OPMonitor`, `xmnat`, RPS agent | partial | the SDK harness now plays live from this box: 12 MB in 90 s from `162.128.66.64:6611` (`protocol.md` 4C.6). The chain is proven end to end: the status cluster's `ServerIP:ServerPort` (`110.239.87.207:6510`, `DevicePort 34567`) is the RPS access server, it assigns the media agent per session, the base64 handshake answers `{"ErrorNum":"200","DestIP":"103.85.122.66","MTU":"1400"}XXEE`, and plain DVRIP follows up to the `OPMonitor` claim (`Ret 100`). The two alternatives stay ruled out with measurements (`protocol.md` 4B): CSS HLS answers `Dev Not Open Cloud Storage`, DSS HLS wants a signature. The RPS access conversation is captured in full (`protocol.md` 4C.7): an `AgentProtocol` `MSG_CLI_NEED_CON_REQ` (`ServiceType RpsAV`, `Step 8`) answered with `AgentServerIp`/`AgentServerPort`, and the tunnel handshake takes the `RpsAV` `SessionId` as its fourth field. `p2p.rs` builds and parses both, pinned by four tests. The transport is in (`crates/core/src/provider/rps.rs`): login to agent assignment to tunnel handshake to DVRIP login all measured from this box in one Rust process, and the `OPMonitor` claim is accepted with `Ret 100` (`protocol.md` 4C.8). Missing: the DHAV key exchange that starts the media, then the feed into the existing fMP4 pipeline |
| Main / sub stream choice | `DevEncodeConfigActivity`, `SDKCONST$StreamType` | done | `stream=0` yields 2304x2592 and `stream=1` yields 640x720 from the same camera, both verified by pulling and probing the capture |
| Multi-channel grid | `CloudServiceChannelListActivity`, `MultiChnCombin` | blocked | the device reports one media profile (`GetProfiles`) and `ChannelNum 1` (DVRIP); the dual lens arrives inside one frame, so a two-viewport grid is a crop of that frame, not a second channel, and a real grid still needs the cloud device list |
| Snapshot | `SDK_AlarmFucntionTypes`, `Fun_MediaRealPlayEx` | partial | the button draws the current frame to a canvas and downloads a PNG; not exercised against the live stream yet |
| Fullscreen | UI-only | done | `F` or the full-screen button puts the stage (or the camera grid) in full screen; exercised in headless Chrome at 1440×900 and 1280×800 for v0.3.0 |
| Two-way audio | `TALK:Java_com_lib_FunSDK_DevStarTalk`, `opTalk` | done | Audible at the camera speaker: the owner heard the test tone from the camera during a talk run ("mendengar suara beep dari kamera", Zhafron, 2026-10-27 16:14Z). Transport, from the shipped SDK (`CProtocolNetIP::NewMediaControlPTLE` 0xf2816c, `NewMediaDataPTLE` 0xf29ee8): `1434` claim (`Ret 100` in **153-477 ms**), `1430` start/stop, `1432` audio out with the 8-byte header `00 00 01 FA / 0E / 02 / len` and G.711 A-law, 320 samples (40 ms) per frame. Measured: an 8 s run **200 frames / 64,000 bytes = 25.0 frames/s, write 0.181 ms mean / 14.770 ms max**, and a 90 s run **2,250 frames / 720,000 bytes = 25.0 frames/s, write 0.052 ms mean / 0.546 ms max** over the vendor relay; the camera also streams its own audio back on `1433` (328-byte frames, 25 a second, 77,952 bytes in 8.96 s), which this build drains and counts. The browser path (`/ws/talk`, `web/talk.js` AudioWorklet at 8 kHz) put 111 frames in 4.40 s with 98.2 % non-silence bytes into a fake peer. CLI: `cargo run -p proview-core --example talk_probe` |
| PTZ | `CInterfaceParser::ToMNetSDKPTZCommand`, `EPTZCMD` | done | ONVIF on `:8899` through `POST /api/device/controls/ptz`: `ContinuousMove` with an explicit `Timeout` (200) plus `SetPreset`/`GotoPreset`/`GetPresets`. Measured on the route: pan +0.15/600 ms moves the frame 20.5 mean-luma points above a 0.4 noise floor, and `GotoPreset 0` returns it to 5.6. `RelativeMove` is refused in code (it stops every device service for 35-45 s), `GetStatus` is unsupported by the firmware, so the logical position lives in the application; only the idempotent preset restore is retried. DVRIP `OPPTZControl` stays the fallback |
| PTZ patrol / tour | `EPTZCMD`, DVRIP `SupportPTZTour` | blocked | ONVIF ver20 PTZ on this firmware exposes no preset-tour operation (`GetNodes` lists only `Wiper start/stop` auxiliaries), and the shipped PTZ transport is ONVIF; the DVRIP fallback reports `SupportPTZTour=true` but its tour call is unverified |
| PTZ on the upper lens | dual-lens IPC `XM535_X6E-WEQ_8M` | blocked | the PTZ node is bound to the lower lens: a `ContinuousMove` shifted the lower lens and left the upper lens at (0,0) px (ZHA-3907). The control route refuses `lens=top` with 409 instead of moving the wrong view |
| Fisheye dewarp | `libfisheye.so` | blocked | not attempted; vendor maths |
| Video decode in browser | `player_hls_flv` in the vendor web client | done | H.264 fMP4 over WebSocket into MSE, verified in headless Chromium (2304x2592, playback clock advancing); a screenshot of the live frame is attached to the issue |
| Live source lifecycle | design | done | one upstream per (device, channel, stream); the init segment plus the newest fragment are kept so a late viewer starts on a keyframe; the source stays warm 60 s after the last viewer leaves and stops itself |
| Multi-viewer fan-out | design | done | two viewers on one stream share a single RTSP session and one transcode; verified with a raw WS client and a browser tab |
| Dual-lens combined view | `SystemFunctionBean$OtherFunction` `MultiLensTwoSensor`, `crates/core/src/lens.rs`, `/api/lens` | done | one upstream stream carries two stacked frames: 2304x2592 = 2×(2304x1296) on the main stream and 640x720 = 2×(640x360) on the sub stream, both measured from the device SPS. The contract reports `stack: vertical` with `cropsVerified: true`. `web/` draws the whole frame from that contract and captions the split with its `countSource` and lens roles; verified in Chromium against `fixtures/dual-lens/` at 720/1000/1280 px, dark and light (ZHA-3911) |
| Dual lens, top half only | `lens.rs` crop 0, `/api/lens?device=&channel=` | done | crop `{x:0,y:0,w:1,h:0.5}` resolves to 2304x1296 and 640x360, unit-tested at both measured sizes. In `web/` the panel takes the crop's aspect and the video renders at 200% height with `top: 0%`, measured 636×716 inside a 636×358 panel in Chromium on the fixture (ZHA-3911) |
| Dual lens, bottom half only | `lens.rs` crop 1, `/api/lens?device=&channel=` | done | crop `{x:0,y:0.5,w:1,h:0.5}` renders with `top: -100%` in the same 200%-height video; the fixture moves only in the bottom half, so a swapped crop is visible in one frame. Verified in Chromium on the fixture, screenshot attached to ZHA-3911 |
| Motorised-lens indicator | `SupportGunBallTwoSensorPtzLocate`, `SupportScaleTwoLens` in `SystemFunctionBean$OtherFunction` | partial | the APK declares a gun+ball sensor pair but never binds a lens index to the motorised one. `/api/lens` reports `ptz: false` on lens 0 and `ptz: true` on lens 1 with `ptzSource: measured_default`, and `web/` labels them "Top lens (fixed)" and "Bottom lens (moving)" from those fields. The source stays a measured default until the device answers for itself (ZHA-3912) |

## Recording / playback

| Feature | Evidence | Status | Note |
|---|---|---|---|
| Device playback | `CloudPlayBackActivity`, `OPPlayBack`, `opSearch` | blocked | needs the device session |
| Cloud recording list | `XMCloudStorage`, `CloudMediaSearchCssHls` | blocked | needs the DSS token (`/reqToken`, `sign` unknown) |
| Clip download | `CCloudStorageDownloadFile` | blocked | follows the two rows above |

## Events / alarms

| Feature | Evidence | Status | Note |
|---|---|---|---|
| Alarm message list | `AlarmMessActivity`, `CMpsClientV2::SearchAlarmInfo`, `AlarmCenter/Header/*` | blocked | cloud message centre, same envelope blocker |
| Alarm push (FCM) | `AlarmPushService`, `SUPPORT_*_PUSH=false` | blocked | vendor FCM sender id + topic authorisation live in the vendor backend |
| Local notification on new event | browser Notification API | blocked | depends on the alarm list source |

## Device management

| Feature | Evidence | Status | Note |
|---|---|---|---|
| Add device by SN | `SnAddDevActivity`, `Add_Device_XMCloud` | done | the account bind is `mdadd` on the same signed envelope, not `/createbind` (that one is the RPS relay agent, `tcp_proxy_agent_client.cpp`): `POST https://rs.xmeye.net/mdadd/v3/<tsid>/<sig>.rs`, header `uuid`/`appKey` plus `Authorization: <access token>`, body AES-128-ECB-PKCS7 of `uuid=&username=&password=&userId=&nickname=&type=&ip=&port=`. `POST /api/devices/bind` and `POST /api/devices/unbind` (`mdrm`) drive it, both rejecting a serial outside 8-32 letters and digits before any vendor call. Measured 2026-10-27: the real serial with its admin password answered `4101 DEV_ALREADY`; the fictional `AASN000000000999` answered `2000` and `GET /api/devices` then listed two cameras; `POST /api/devices/unbind` answered `2000` and the list went back to one; `uuid=no` answered `4104 DEV_UUID_FAIL`. The cloud accepts a well-formed serial that is not a device, so `msdlistCode` stays the check. See `protocol.md` 10E |
| Add device by QR | `DevQrCodeDlg`, `assets/key/qrkey.txt` | done | the payload codec is `XEnCodeStr`/`XDeCodeStr` from libFunSDK: `%03d` of each byte plus the offset, a digit shift by the digit sum of the offset, base62 packing, a per-character shift keyed `DecInfoEncode`, and the trailing `z`. `crates/core/src/cloud/bind.rs` reproduces it byte for byte against the vendor function (10 vectors, offsets 0-743) and `POST /api/devices/bind/qr` accepts either a bare blob (`sn,user,pwd,type,time`) or the share link `https://d.xmeye.net/CSee?shareInfo=<blob>` over `{devType,userId,loginName,pwd,devId,shareTimes}`. Measured 2026-10-27: the vendor blob for the real serial parsed to `7f92970ddf5282743hrc` and answered `4101`; a payload for `AASN000000000777` bound at `2000`, appeared in `GET /api/devices`, and `mdrm` removed it; `not-a-code` was refused with `bad_qr` |
| Add device over AP mode | `APConfigNetWorkTip`, `AddDeviceActivity`, `SearchDevApActivity`, `QuickConfigResultActivity`, `CConfigAPDev`, `concrete_package`, `xmmulticastconfigsend` | partial | protocol recovered from the binary and implemented in the server: DVRIP `NetWork.Wifi` write to `192.168.10.1:34567` as `admin` with an empty password (message 1040, 2000 ms, `WPA2`/`AES` or `OPEN`/`NONE` chosen by passphrase presence), and the app's fallback sender `xmmulticastconfigsend`: UDP `255.255.255.255:8001`, one datagram per symbol whose length is the symbol, symbols `0x80 + 8k + 64g + ((V >> 3k) & 7)` over three record bytes with a short tail group, a 2048-byte payload of ten `'e'` then the `S:`/`P:`/`ip:` string, 20 ms per symbol, a 500-byte datagram every 51 ms, 5 ms per round, run until stopped. Routes `POST /api/apmode/plan`, `POST /api/apmode/provision` (background run, one at a time), `GET`/`DELETE /api/apmode/provision`. Still `partial` because the physical step has not been run on a camera: reset it, join its access point, provision, confirm it joins the home network (`docs/apmode.md` section 6) |
| Rename device | `IXMAccount::EditDevice` | blocked | cloud |
| Favourite / group | `GroupDeviceListResp`, `DEVICE_SHARE_ENABLE=true` | blocked | cloud |
| Device settings pages | `DevBasicSettingActivity`, `DevEncodeConfigActivity`, `DevRecordSettingActivity`, `DevStorageSettingActivity` | partial | read side is live: `GET /api/device/controls/settings` returns `GetDeviceInformation`, the profile, `GetVideoEncoderConfiguration` (H264 3072x2048, 12 fps, 1536 kbps, GOP 36), `GetImagingSettings` and `GetNetworkInterfaces`. DVRIP settings routes cover white light, picture flip and record mode. Write side for encoder/network/storage is not wired (permanent config writes need an explicit owner decision) |
| Change device password | `DevPsdManageActivity`, `IXMAccount::ChangeDevLoginPwd` | blocked | device session |
| Share device to another account | `com.xworld.activity.share.*`, `.rs` share API | blocked | cloud account model |

## What this project adds that the APK does not

| Feature | Why |
|---|---|
| Desktop browser UI | the point of the project |
| Server-side vendor session | keeps the vendor token out of JavaScript |
| Provider seam (`fixture` / `cloud` / `local`) | lets the UI be exercised without an account, and picks LAN or cloud per profile |
| Mode and fallback reporting | the UI can say which provider answered instead of failing silently |

## Blockers, consolidated

1. **Account login without the vendor library** — the envelope itself is proven
   (signed `/v3/<tsid>/<sig>.rs`, `protocol.md` 3A), but producing the per-request
   `sig` and the encrypted body needs key material that only lives in
   `libFunSDK.so`. Everything in “Platform core” and most of the cloud rows depend
   on a client that can do it without the vendor library.
2. **Device transport** — RTSP (554) or DVRIP (34567) reachability to the
   camera, or the P2P path (`xmnat` + `rps`/KCP). Live view, playback, PTZ and
   audio all sit behind it.
3. **DSS media token** — `/reqToken` body is fully known
   (`{sn,user,pass,service,uuid,appkey,tm,sign}`) but `sign` derivation and the
   accepted `service` value are not.
