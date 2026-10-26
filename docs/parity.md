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

## Live view

| Feature | Evidence | Status | Note |
|---|---|---|---|
| Device reachability probe | `CNetDevice`, DVRIP login | partial | route `POST /api/device/probe` is implemented and tested only for the unconfigured case (400). No real device has been contacted yet, so the handshake itself is unproven |
| Recorded-clip playback path | `Fun_MediaDownloadM3u8ByUrl`, `CMediaFile` | partial | Annex-B clip → fMP4 is served and plays in Chromium (verified with ffprobe + in-browser decode); the live transport feeding it does not exist yet |
| Live video | `DeviceMediaActivity`, `CMediaChannel`, `OPMonitor` | blocked | needs the device session: LAN RTSP/DVRIP on 554/34567, or cloud DSS/HLS |
| Main / sub stream choice | `DevEncodeConfigActivity`, `SDKCONST$StreamType` | blocked | follows the transport |
| Multi-channel grid | `CloudServiceChannelListActivity`, `MultiChnCombin` | blocked | follows the transport |
| Snapshot | `SDK_AlarmFucntionTypes`, `Fun_MediaRealPlayEx` | blocked | client-side frame grab once video runs |
| Fullscreen | UI-only | blocked | follows the transport |
| Two-way audio | `TALK:Java_com_lib_FunSDK_DevStarTalk`, `opTalk` | blocked | audio transport (`g711a/g711u`) is a separate channel |
| PTZ | `CInterfaceParser::ToMNetSDKPTZCommand`, `EPTZCMD` | blocked | follows the device session |
| Fisheye dewarp | `libfisheye.so` | blocked | not attempted; vendor maths |
| Video decode in browser | `player_hls_flv` in the vendor web client | partial | HLS path wired in the UI; no source yet |

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
| Device settings pages | `DevBasicSettingActivity`, `DevEncodeConfigActivity`, `DevRecordSettingActivity`, `DevStorageSettingActivity` | blocked | device config channel (`CDevProtocol`, `opGetFile`/`set`) |
| Change device password | `DevPsdManageActivity`, `IXMAccount::ChangeDevLoginPwd` | blocked | device session |
| Share device to another account | `com.xworld.activity.share.*`, `.rs` share API | blocked | cloud account model |

## What this project adds that the APK does not

| Feature | Why |
|---|---|
| Desktop browser UI | the point of the project |
| Server-side vendor session | keeps the vendor token out of JavaScript |
| Provider seam (`fixture` / `proview`) | lets the UI be exercised without an account |

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
