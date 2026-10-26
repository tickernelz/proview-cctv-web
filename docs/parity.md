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
| Cloud account login | `com.xworld.activity.login.*`, `IXMAccount::AccountLogin`, `XM_Video_Login_XMAccount` | blocked | request shape known (`POST /webservice/app/user/logInByAccount`, cookie session); CSS JSON envelope + password digest not solved |
| Session kept server side | design | done | private `pvw_session` cookie → in-memory vendor session; vendor token never reaches the browser |
| Session expiry handling | design | partial | server expires and returns 401; UI shows the error but does not re-prompt silently |
| Remember account | `SplashActivity`, `loginExist`, `savePwdToken` strings | blocked | needs the flow in the previous rows |
| Logout | `IXMAccount::AccountUnBindingWX`, `/webservice/app/user/logOut` | partial | local logout works; vendor-side logout call not wired |
| Device list | `DeviceFragment`, `com.xworld.data.GroupDeviceListResp`, `IXMAccount::GetDevList` | blocked | same cloud blocker |
| Device online status | `pub-status.secu100.net:7700-7706`, `IXMCloud::GetDevsStatus` | blocked | same cloud blocker |

## Application shell

| Feature | Evidence | Status | Note |
|---|---|---|---|
| Self-hosted HTTP server | `crates/server`, `--listen` | done | 8 target binaries in release `nightly-1` (linux gnu/musl x86_64+aarch64, windows msvc x86_64+aarch64, macOS x86_64+aarch64) |
| Native desktop shell (Tauri 2) | `src-tauri/` | done | CI builds it on ubuntu-22.04, windows-latest, macos-latest; release `nightly-1` carries msi/nsis, dmg (x64/arm64/universal), AppImage/deb/rpm |
| Installer signing | — | blocked | no Apple Developer or Windows signing certificate (decision 2026-10-26); artefacts ship unsigned with `SHA256SUMS` |
| Desktop app run on a real desktop | — | blocked | built and linked in CI, never launched on a graphical session; needs a person to click through |
| Vendor-credential storage in OS keychain | requirement | partial | `src-tauri/src/keychain.rs` stores one JSON blob per profile through `keyring` (macOS Keychain, Windows Credential Manager, Secret Service), read at startup and writable with `proview-desktop --store-credentials`; the CLI round trip is exercised, the desktop window is not |
| Provider seam (`cloud` / `local`) | requirement | done | `crates/core/src/provider/`: one `Provider` trait for device list, live, playback, alarms and settings, with `CloudProvider` (default) and `LocalProvider` behind `Router` |
| Mode selection and fallback | requirement | done | `GET/POST /api/config/mode`; desktop persists the choice in `app_data_dir/mode.json`, server keeps it per session; local without a host or with an unreachable host falls back to cloud and reports why |

## Live view

| Feature | Evidence | Status | Note |
|---|---|---|---|
| Device reachability probe | `CNetDevice`, DVRIP login | done | `POST /api/device/probe` logs in to the IPC at `192.168.0.100:34567` with the vendor "sofia" 8-character hash: `Ret: 100`, `SessionID 0x00000017`, `DeviceType "IPC "`, `ChannelNum 1`. An empty or 32-hex hash is answered `Ret: 203`, which is where the old `ret 203` report came from |
| Recorded-clip playback path | `Fun_MediaDownloadM3u8ByUrl`, `CMediaFile` | partial | Annex-B clip → fMP4 is served and plays in Chromium (verified with ffprobe + in-browser decode); the live transport feeding it does not exist yet |
| Local RTSP live view | RTSP client inside `libFunSDK.so` | done | `crates/core/src/rtsp.rs` (Digest session, SETUP/PLAY over TCP interleaved) plus `media/rtp.rs` and `media/hevc.rs` pull the IPC at `192.168.0.100:554`: 72 access units in 6 s, all decodable by ffmpeg, played in Chromium through the fMP4/WebSocket hub |
| Live video over cloud P2P | `DeviceMediaActivity`, `CMediaChannel`, `OPMonitor`, `xmnat` | blocked | the portal bundle gave up the login encoder (AES-128-ECB, key in webpack module `c71e`, `POST https://api.xmeye.net/api/transfer/login/v1` with a `Code-Token` captcha), but the PROVIEW account is a white-label account and that global portal answers `wrong param`; the white-label path runs through `IXMAccount` inside `libFunSDK.so` and is not solved |
| Main / sub stream choice | `DevEncodeConfigActivity`, `SDKCONST$StreamType` | done | `stream=0` yields 2304x2592 and `stream=1` yields 640x720 from the same camera, both verified by pulling and probing the capture |
| Multi-channel grid | `CloudServiceChannelListActivity`, `MultiChnCombin` | blocked | the device reports one media profile (`GetProfiles`) and `ChannelNum 1` (DVRIP); the dual lens arrives inside one frame, so a two-viewport grid is a crop of that frame, not a second channel, and a real grid still needs the cloud device list |
| Snapshot | `SDK_AlarmFucntionTypes`, `Fun_MediaRealPlayEx` | partial | the button draws the current frame to a canvas and downloads a PNG; not exercised against the live stream yet |
| Fullscreen | UI-only | partial | `requestFullscreen` wired on the stage; not exercised in the verification run |
| Two-way audio | `TALK:Java_com_lib_FunSDK_DevStarTalk`, `opTalk` | blocked | recorded offer: RTSP SDP carries `m=audio 0 RTP/AVP 8` (`PCMA/8000`, `trackID=4`) but advertises no backchannel, and `GetCapabilities` on 8899 exposes no audio service. DVRIP reports `SupportTwoWayVoiceTalk=true`, so the missing piece is a capture path in this server, not the device |
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
| Add device by SN | `SnAddDevActivity`, `Add_Device_XMCloud` | blocked | cloud bind (`/createbind`, `/listbind`) |
| Add device by QR | `DevQrCodeDlg`, `assets/key/qrkey.txt` | blocked | same cloud bind |
| Add device over AP mode | `APConfigNetWorkTip`, `AddDeviceActivity` | blocked | device Wi-Fi AP provisioning, no cloud needed but needs the device |
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

1. **Account login envelope** — `CssCenter` JSON message type and body for
   `logInByAccount`, plus the password digest. Everything in “Platform core” and
   most of the cloud rows depend on this one fact.
2. **Device transport** — RTSP (554) or DVRIP (34567) reachability to the
   camera, or the P2P path (`xmnat` + `rps`/KCP). Live view, playback, PTZ and
   audio all sit behind it.
3. **DSS media token** — `/reqToken` body is fully known
   (`{sn,user,pass,service,uuid,appkey,tm,sign}`) but `sign` derivation and the
   accepted `service` value are not.
