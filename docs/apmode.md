# AP-mode provisioning (row: Add device over AP mode)

Recovered from the shipped app, not from vendor documentation: every claim below
names the symbol or instruction that carries it. Artifacts stay outside this
repository.

| Artifact | Value |
|---|---|
| APK | `com.proview.cctv` 1.0.4 (versionCode 10403), `proview-cctv_1.0.4_apkpure.apk`, SHA-256 `b347696d8f810f0fa3e3f19118e06d25a760307a1ea28dda55efe709cfefaaaf` |
| Native SDK | `arm64-v8a/libFunSDK.so`, 33,983,560 B, SHA-256 `04e9c0f53ca7a0aac12e9bac4c15ed2e7543b5ab8deaeeef0bf2a223fd85249a`, C++ symbols present |
| Decompiler | jadx 1.5.3 (APK), GNU objdump 2.46.1 + radare2 6.2.4 (aarch64 disassembly) |
| Artifact notes | `~/re/proview-cctv/ZHA-4016/note.md`, `~/re/proview-cctv/ZHA-4016/out/*.asm` |

## 1. What the app does

Two transports carry the credentials. The SDK starts both; either one is enough
for a camera that is listening on that transport.

```
AddDeviceActivity                     (com/xworld/activity/adddevice/AddDeviceActivity.java)
  onMenuItem "AP mode"  -> SearchDevApActivity
SearchDevApActivity
  V8()  builds SDBDeviceInfo{login "admin", password "", port 34567}
        FunSDK.DevGetConfigByJson(handle, "192.168.10.1", "SystemInfo", 1024, -1, ...)
  OnFunSDKResult 5126  LAN search rows, keeps the one whose HostIP == 192.168.10.1
  OnFunSDKResult 5128  login result; -11301 / -11318 raise the password prompt
DevWirelessNetWorkSettingActivity     (user picks the home SSID and types the key)
  JsonConfig.WIFI_AP, command id 1020, scans the access points
  AP flag f31684S -> re-login 192.168.10.1 with the device local credentials
QuickConfigResultActivity
  line 428: FunSDK.DevStartWifiConfigByAPLogin(handle, "192.168.10.1", ssid, pwd, -1)
  line 605/1000/1287: FunSDK.DevStopWifiConfig() on cancel and on result 5127
```

`EUIMSG.DEV_SET_WIFI_CFG = 5127` is the callback the app waits for; the same
number is stored as the message id by `CConfigAPDev::Start_Wifi`
(`0xb28254`, `mov w9, #0x1407`).

## 2. Native chain

| Symbol | Address | Role |
|---|---|---|
| `Java_com_lib_FunSDK_DevStartWifiConfigByAPLogin` | `0xdd6cb4` | JNI entry |
| `FUN_DevStartWifiConfigByAPLogin` | `0xbf10ec` | calls the Ex form with `apLogin = 1` |
| `FUN_DevStartWifiConfig` | `0xbf10a0` | same call with `apLogin = 0` |
| `FUN_DevStartWifiConfigEx` | `0xbf0cfc` | copies SN into 36 bytes, SSID into 128 bytes, then calls the class |
| `CConfigAPDev::StartWifiConfig` | `0xb28068` | singleton, then `Start_Wifi` |
| `CConfigAPDev::Start_Wifi` | `0xb28218` | memcpy of the 284-byte `SDK_NetWifiConfig` to `this+0x610`, reads `SystemInfo` and `NetWork.Wifi`, 8000 ms |
| `CConfigAPDev::OnMsg` | `0xb2960c` | merges the answer and writes `NetWork.Wifi` back, 2000 ms |
| `CConfigAPDev::ThreadWork_XMSend` | `0xb289a4` | starts the datagram transmitter |
| `xmmulticastconfigsend` | `0xf6ad3c` | builds one packet and sends it; the only sender the app reaches |
| `xmmulticastconfigsend2` / `xmconfigsend` | `0xf6b2c4` / `0xf6be90` | the sibling branches, chosen by `this+560..563` |
| `parseWifiInfo` | `0xf6a9ec` | reads `S:`, `P:`, `T:` out of the info string |
| `concrete_package` | `0xf6d828` | builds the word array |
| `xmbroadcastconfigsend` | `0xf6e164` | a test helper: its only caller is `xtest_broadcastSend` (`0xf6e55c`), which nothing calls |
| `xmconfigstop` | `0xf6c910` | clears the global stop flag at `0x2075ad0` |

`CConfigAPDev` also runs `Work_ap_net_wifi_cfg` (`0xb2b8e4`), which calls
`SearchDevicesBySn` (`0xb2b558`) -> `CDeviceBase::SearchDevices` (`0xb7e6e4`)
and hands the device to `OnConfigApDev` (`0xb29c90`). That is the LAN search of
`192.168.10.1` again, used to know when the camera leaves AP mode.

The datagram branch is the one the app takes: the app's cancel and result paths
call `DevStopWifiConfig`, which clears the `xmconfigstop` flag the send loop
reads, and the sender it starts is `xmmulticastconfigsend`. The
`xmbroadcastconfigsend` variant is test scaffolding in the vendor build.

## 3. Transport A — DVRIP config write

The reachable path, and the one a browser server can drive without raw sockets:

```
TCP 192.168.10.1:34567
  login            user "admin", password "" on a camera that was just reset
  read             {"Name":"NetWork.Wifi","SessionID":"0x…"}          message 1042
  write            {"Name":"NetWork.Wifi","SessionID":"0x…",
                    "NetWork.Wifi":{ … }}                              message 1040, 2000 ms
  answer           Ret 100, and the app reports through callback 5127
```

Fields of `NetWork.Wifi` (names from the app's `com.lib.sdk.bean.NetworkWifi`
and from the strings at `0x4b5780` `EncrypType`, `0x4bdafc` `KeyType`,
`0x4cf09d` `Enable`, `0x4cf0a4` `GateWay`, `0x4d7118` `SSID`,
`0x500fc8` `Keys`, `0x500fcd` `Submask`, `0x511d39` `NetType`,
`0x53ba6d` `HostIP`, `0x56d880` `Auth`):

| Field | Meaning |
|---|---|
| `Enable` | take the wireless interface up |
| `SSID` | home network name |
| `Keys` | passphrase, clear text on this transport |
| `Auth` | `WPA2` when the passphrase is non-empty, `OPEN` otherwise |
| `EncrypType` | `AES` when the passphrase is non-empty, `NONE` otherwise |
| `NetType`, `Channel`, `HostIP`, `Submask`, `GateWay`, `KeyType` | kept from the read |

Which member takes which value is read from the object's own constructor,
not from the order of the string table: `CConfigAPDev` builds
`JsonObj::NetWork_Wifi` at `this+0x2d0` (`0xb26e64`, `bl 0xb271ac`), and that
constructor names member `+0x40` `Auth` (`0xb271ec`, string `0x56d880`) and
member `+0x108` `EncrypType` (`0xb2723c`, string `0x4b5780`). `OnMsg` then writes
`this+0x310` (= `0x2d0+0x40`) and `this+0x3d8` (= `0x2d0+0x108`):

| passphrase | `Auth` (`+0x310`) | `EncrypType` (`+0x3d8`) |
|---|---|---|
| at least one character | `WPA2` (`0xb298ac`, string `0x5097e2`) | `AES` (`0xb298c0`, string `0x5097e7`) |
| empty | `OPEN` (`0xb298d8`, string `0x4e7c24`) | `NONE` (`0xb298ec`, string `0x4d70e9`) |

The camera measured in 3.1 agrees: `EncrypType "AES+TKIP"` is the cipher name and
`Auth "WPA2PSK"` the authentication mode.

Login detail: on a factory-reset camera the SDK logs in with `admin` and an
empty password (`SearchDevApActivity.V8`, `com.basic.b.n(st_5_loginPsw, "")`).
DVRIP itself hashes the password before sending it (sofia hash, section 6 of
`docs/protocol.md`), so an empty password is sent as the hash of the empty
string, not as an empty field.

### 3.1 Measured on a real IPC (2026-10-27, 192.168.0.100:34567)

The camera on the owner's LAN, firmware `V5.01.R02.G51A07F3.10010.346732.S.ONVIF
21.06`, proves the shape this implementation uses:

| Step | Result |
|---|---|
| login `admin` + device password | `Ret 100`, session 563-570, `DeviceType "IPC"`, one channel, 21-31 ms |
| read `NetWork.Wifi` | `Ret 100` in 163-170 ms, reply keys `Name`, `Ret`, `SessionID`, **`NetWork.Wifi`** — a flat key, not a nested `NetWork.Wifi` object |
| block on that firmware | `EncrypType "AES+TKIP"`, `Auth "WPA2PSK"`, `NetType "Infra"`, `KeyType 1`, `Enable true` |
| write the same block back, flat shape | `Ret 100` in 201 ms |
| read back | identical block: SSID, cipher, auth and passphrase length unchanged |

Two consequences the code follows: the read is parsed from the flat
`NetWork.Wifi` key first (nested accepted as a fallback), and a write keeps the
`EncrypType`/`Auth` names the firmware already has **only while the request
stays in the same class**. An empty passphrase writes `Auth "OPEN"` and
`EncrypType "NONE"`, a secured request against an open block writes
`Auth "WPA2"` and `EncrypType "AES"`, and a camera that already says
`AES+TKIP`/`WPA2PSK` keeps those names because rewriting them would move it to
terms its firmware does not use.

The run took a backup of the read reply first, replayed it unchanged, compared
the read-back, and would have written the backup again on any mismatch. It did
not change the camera's network settings.

## 4. Transport B — UDP symbol stream

### 4.1 The record the symbols carry

`concrete_package` builds a record in a stack buffer (`0xf6d8f4`-`0xf6dc04`)
and then encodes it, three bytes at a time:

| Offset | Bytes | Value |
|---|---|---|
| 0 | 1 | total record length, `ssid_len + psk_len + 7` |
| 1 | 1 | `0x41` (`'A'`), constant |
| 2 | 1 | SSID length |
| 3 | 1 | passphrase length |
| 4 | ssid_len | SSID bytes |
| 4+ssid_len | psk_len | passphrase bytes |
| … | 1 | fourth octet of the `ip:` value in the info string, `0` when the key is absent (`0xf6dacc`, fed from `0xf6afe4`/`0xf6b04c`) |
| … | 2 | 16-bit sum of every byte above, little endian (`0xf6daf8`-`0xf6db74`) |

Device-side limits are visible in the same struct: SSID 34 bytes
(`cfg+6` … `cfg+40`), passphrase 132 bytes (`cfg+48` … `cfg+180`).

### 4.2 Encoding

Each group of three record bytes becomes eight 16-bit symbols, and every symbol
carries its own position: the slot constant (`0x80`, `0x88`, `0x90`, `0x98`,
`0xa0`, `0xa8`, `0xb0`, `0xb8` at `0xf6dc2c`, `0xf6dc54`, `0xf6dc80`,
`0xf6dce8`, `0xf6dd18`, `0xf6dd48`, `0xf6ddb0`, `0xf6dde0`) is combined with
the group's word index shifted left three (`orr w9, w9, w12, lsl #3`, where
`w12 = (i*8)/3 = 8g` from the `umull` at `0xf6dbac`):

```
words[0..3] = 0x0514 three times                       (0xf6d904-0xf6d918)
for every full group g of three bytes:
    if 8g == 32:  emit 0x0515 three times              (0xf6dbc4-0xf6dc04)
    V = b0 | b1 << 8 | b2 << 16
    for k in 0..8:  emit 0x80 + 8k + 64g + ((V >> 3k) & 7)
tail, only when the record length is not a multiple of three (0xf6de18-0xf6df94):
    one byte  -> three symbols, two bytes -> six symbols, no 0x0515 check
```

The word count is computed, not counted: `3 + off + (8L + 2) / 3`, where `off`
is `3` when a boundary marker was emitted and `0` otherwise
(`0xf6dfa0`-`0xf6dfc8`).

Worked example, SSID `CAM-LAB`, passphrase `rahasia123` (record 24 bytes):

| Quantity | Value |
|---|---|
| groups | 8, no tail |
| words | 70 |
| first eight symbols | `80 8b 94 98 a4 ae b1 b8` |
| boundary marker | packet words 35..37 (`0x0515` three times) |
| largest symbol | `0x278` |

A 10-byte record gives 30 symbols and no marker; a 14-byte record gives 41 and
no marker; 23 bytes give 68 with one marker; 47 bytes give 132.

### 4.3 The sender

`xmmulticastconfigsend` (`0xf6ad3c`) prepares one socket and repeats until
`xmconfigstop` clears the global flag at `0x2075ad0`:

```
socket(AF_INET, SOCK_DGRAM, 0)                       0xf6af54
setsockopt(SOL_SOCKET=1, SO_BROADCAST=6, 1, 4)       0xf6af7c
sin_family = AF_INET, sin_port = 0x411f (8001)       0xf6af3c, 0xf6af40
sin_addr   = 0xffffffff, written once                0xf6af4c
per round, per word i (loop 0xf6b0e0 with the counter at [sp+0x40]):
    sendto(sock, payload, words[i], 0, &broadcast, 16)   0xf6b178
    usleep(20000)                                        0xf6b18c
after the last symbol of the round (0xf6b1a8, reached only when the counter passes the word count):
    if 51 ms passed since the last 500-byte send (0xf6b1cc, subs #0x33):
        sendto(sock, payload, 500, 0, &broadcast, 16)     0xf6b228
    usleep(5000)                                          0xf6b26c
```

The destination never changes: the `ip:` value in the info string is read
(`0xf6adec`, `inet_addr`) but ends up only in the record's fourth octet, while
`sin_addr` stays `0xffffffff` from `0xf6af4c`. The 25000 ms constant at
`0xf6b0a4` is stored and never compared, so the loop is bounded by the stop
flag alone; there is no round cap.

The datagram payload is a 2048-byte stack buffer at `sp+0x41c`: zeroed
(`0xf6ae40`), the first ten bytes set to `0x65` (`'e'`, `0xf6ae74`), then the
info string copied in at offset ten with a 512-byte limit (`0xf6ae90`). A
datagram of length `n` therefore carries those first `n` bytes: ten `'e'`,
then `S:<ssid>P:<password>ip:<address>`, then zeros. Our implementation sends
exactly that buffer prefix.

## 5. What the physical test still has to settle, and why this box cannot

The DVRIP half is measured on hardware (section 3.1). The datagram half needs a
client that has **joined the camera's own access point**, and this workstation
cannot do that:

| Measurement | Value |
|---|---|
| network interfaces | `lo`, `eth0` 172.23.199.41/20, `tailscale0`, docker bridges — **no `wlan*` device** |
| `192.168.10.1:34567` (AP-mode camera) | connection timeout after **4008 ms** |
| `192.168.10.1:80` | connection timeout after **4008 ms** |
| the reachable camera `192.168.0.100` | `34567` open in 6 ms, `554` open in 6 ms |

So the remaining step needs a host with a Wi-Fi adapter next to the camera, not
a decision: there is no interface to join `XM-…` with, and the AP address is
not routed from here. What that test has to settle:

1. Whether the AP-mode firmware decodes the stream purely from datagram length,
   or also parses the payload text.
2. Whether the camera accepts the DVRIP write before another client joins its
   access point, or only after (the app joins the AP first and then logs in).
3. What the `0x0515` triple at body word 32 means, and whether records long
   enough to pass it twice behave as the literal single check suggests.
4. Timing tolerance: 20 ms per symbol, a 500-byte datagram whenever 51 ms have
   passed, and 5 ms between rounds are the SDK's values, not a measured minimum.

## 6. Physical test steps

Ten minutes, one camera, one laptop on the same Wi-Fi:

1. Reset the camera and hold the reset button until it announces the AP. Its
   access point appears as `XM-…` / `GW_AP_…` and the camera answers on
   `192.168.10.1`.
2. Join that access point from the laptop. Do not disable the laptop's own
   Wi-Fi adapter; the AP is what we are provisioning through.
3. Run the server, sign in, and ask for the plan:
   `POST /api/apmode/plan {"ssid":"<home>","password":"<key>"}`. It answers the
   record size, the symbol count and the per-round totals. Expected for a
   7-character SSID and a 10-character key: 24-byte record, 70 symbols and
   `datagrams_per_round 71` — the 70 symbols plus one 500-byte trailer that the
   sender emits after the last symbol of every round.
4. `POST /api/apmode/provision {"ssid":"<home>","password":"<key>","mode":"dvrip"}`.
   Expected: `ret 100` from message 1040, `read_back true` when the camera
   echoes the SSID, and `elapsed_ms` inside the budget that is actually
   enforced: a 3 s connect timeout, up to four requests at 2 s each (login,
   read, write, read-back) and one more for logout, so **about 11 s worst
   case** and 0.4-1 s when the camera answers. On a reachable IPC this path is already measured: 163-170 ms for the
   read, 201 ms for the write, `Ret 100` and an identical read-back
   (section 3.1). What is not yet measured is the write **through the camera's
   own access point**, which is what step 2 sets up.
5. If the DVRIP path is refused, run `"mode":"broadcast"` and poll
   `GET /api/apmode/provision`: about 1.41 s per round (70 symbols at 20 ms plus
   the 5 ms round gap and one 500-byte trailer after the last symbol). Stop it
   with `DELETE /api/apmode/provision`. Watch the camera's LED: a successful
   receive usually stops the AP broadcast.
6. Bring the laptop back to the home network. The camera should join it within a
   minute, appear in the LAN search, and answer DVRIP on its new address.
7. Record in the issue: which mode worked, the `ret` value, the time from the
   last datagram to the camera joining, and the new address.

Until step 7 has been run on real hardware the write through the camera's own access point stays unmeasured; the parity row is `done` under Zhafron's 2026-10-27 23:29 UTC decision and keeps that boundary.

## 7. Server surface

| Route | Body | Answer |
|---|---|---|
| `POST /api/apmode/plan` | `{ssid, password}` | record size, symbol count, per-round totals; no traffic and no credential echo |
| `POST /api/apmode/provision` | `{ssid, password, mode?, device_password?, rounds?}` | DVRIP `ret`/`read_back`/`elapsed_ms` or its error, plus the broadcast run status |
| `GET /api/apmode/provision` | — | rounds, datagrams, bytes, elapsed, error, whether it is still running |
| `DELETE /api/apmode/provision` | — | asks the sender to stop at the next symbol (the app's `DevStopWifiConfig`) |

Limits the caller can hit:

| Limit | Value |
|---|---|
| SSID / passphrase | 34 / 132 bytes (`ApError::SsidTooLong`, `PasswordTooLong`) |
| record per packet | `MAX_WORDS` 186 symbols, i.e. an SSID plus passphrase of about 60 bytes; longer credentials answer `ApError::RecordTooLong` at plan time |
| one run at a time | a second `POST` while a run is sending answers `409` |
| run length | `rounds` or the 180 s ceiling, which is about 127 rounds at the app's timing |

The target is fixed: DVRIP `192.168.10.1:34567` as `admin` with an empty
password unless `device_password` is given, and UDP `255.255.255.255:8001`.
Nothing in the body can point the server at another host, and the configured
home-camera password is never used here. `mode` is `both` (default), `dvrip` or
`broadcast`; in `both` a DVRIP failure is reported and the datagram run still
starts, because either transport is enough for the camera. One run at a time:
a second `POST` while a run is sending answers `409`. The sender also stops
itself after 180 s.

Verification commands (no camera needed):

```
~/re/proview-cctv/build-slot.sh cargo test -p proview-core --lib apmode
~/re/proview-cctv/build-slot.sh cargo test -p proview-server --lib apmode
~/re/proview-cctv/build-slot.sh cargo test -p proview-server --test api apmode
```

## 8. Re-checking the evidence

```
aarch64-linux-gnu-objdump -d --start-address=0xf6d828 --stop-address=0xf6e164 \
  ~/re/proview-cctv/native/arm64-v8a/libFunSDK.so      # concrete_package
aarch64-linux-gnu-objdump -d --start-address=0xf6ad3c --stop-address=0xf6b2c4 \
  ~/re/proview-cctv/native/arm64-v8a/libFunSDK.so      # xmmulticastconfigsend
aarch64-linux-gnu-objdump -d --start-address=0xb28218 --stop-address=0xb283a8 \
  ~/re/proview-cctv/native/arm64-v8a/libFunSDK.so      # Start_Wifi
```
