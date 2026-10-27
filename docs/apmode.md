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
| `xmmulticastconfigsend` / `xmconfigsend` | `0xf6ad3c` / `0xf6be90` | build one packet, send it |
| `parseWifiInfo` | `0xf6a9ec` | reads `S:`, `P:`, `T:` out of the info string |
| `concrete_package` | `0xf6d828` | builds the 1300-byte packet |
| `xmbroadcastconfigsend` | `0xf6e164` | the socket loop: one datagram per symbol |

`CConfigAPDev` also runs `Work_ap_net_wifi_cfg` (`0xb2b8e4`), which calls
`SearchDevicesBySn` (`0xb2b558`) -> `CDeviceBase::SearchDevices` (`0xb7e6e4`)
and hands the device to `OnConfigApDev` (`0xb29c90`). That is the LAN search of
`192.168.10.1` again, used to know when the camera leaves AP mode.

## 3. Transport A — DVRIP config write

The reachable path, and the one a browser server can drive without raw sockets:

```
TCP 192.168.10.1:34567
  login            user "admin", password "" on a camera that was just reset
  read             {"Name":"NetWork.Wifi","SessionID":"0x…"}          message 1042
  write            {"Name":"NetWork.Wifi","SessionID":"0x…",
                    "NetWork":{"Wifi":{ … }}}                          message 1040, 2000 ms
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
| `EncrypType` | `WPA2` when the passphrase is non-empty, `OPEN` otherwise |
| `Auth` | `AES` when the passphrase is non-empty, `NONE` otherwise |
| `NetType`, `Channel`, `HostIP`, `Submask`, `GateWay`, `KeyType` | kept from the read |

The `WPA2`/`OPEN` and `AES`/`NONE` pair comes from `CConfigAPDev::OnMsg`:
`0xb298b0` writes `WPA2` (`0x5097e2`) and `AES` (`0x5097e7`) into the two
string members when the stored passphrase length is at least one, and
`0xb298e4` writes `OPEN` (`0x4e7c24`) and `NONE` (`0x4d70e9`) when it is zero.

Login detail: on a factory-reset camera the SDK logs in with `admin` and an
empty password (`SearchDevApActivity.V8`, `com.basic.b.n(st_5_loginPsw, "")`).
DVRIP itself hashes the password before sending it (sofia hash, section 6 of
`docs/protocol.md`), so an empty password is sent as the hash of the empty
string, not as an empty field.

## 4. Transport B — UDP symbol stream

The path for a camera that is not reachable by IP yet. Look at
`xmbroadcastconfigsend`:

```
socket(AF_INET, SOCK_DGRAM, 0)                     0xf6e314
setsockopt(SOL_SOCKET=1, SO_BROADCAST=6, 1, 4)     0xf6e330
sin_family = AF_INET                               0xf6e2fc
sin_port   = 0x411f stored as raw bytes -> 8001     0xf6e300
sin_addr   = 0xffffffff (255.255.255.255)          0xf6e308
sendto(sock, &packet[352], length = packet_word[i]) 0xf6e46c
usleep(1000)                                       0xf6e47c
sendto(sock, &packet[352], 500)   once per round    0xf6e4e8
usleep(100000)                    between rounds     0xf6e4f4
rounds < 50                       while the run flag is set   0xf6e3a4
```

`xmmulticastconfigsend` (`0xf6ad3c`) does the same but takes the destination
from an `ip:` key inside the info string (`Info_GetValue(info, "ip:", buf)` at
`0xf6adc8`, `inet_addr` at `0xf6adec`), which is the multicast/unicast variant;
the port and the packet are identical.

So one logical message is **one datagram per symbol**, and the datagram's
**length** is the symbol. Content is the packet window at offset 352, which the
receiver can read but does not need in order to parse the length.

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
| … | 1 | fourth octet of the destination IP (`cfg+195`, loaded at `0xf6dacc`) |
| … | 2 | 16-bit sum of every byte above, little endian (`0xf6daf8`-`0xf6db74`) |

Device-side limits are visible in the same struct: SSID 34 bytes
(`cfg+6` … `cfg+40`), passphrase 132 bytes (`cfg+48` … `cfg+180`).

### 4.2 Encoding

The packet is 1300 bytes, read as 650 little-endian `u16` words.

```
words[0..3]  = 0x0514   three times            (0xf6d904-0xf6d918)
for each group g of three record bytes, in order:
    if 8*g == 32:  emit 0x0515 three times      (0xf6dbc4-0xf6dc04)
    V = b0 | b1 << 8 | b2 << 16                 (partial last group: zero bytes)
    for k in 0..8:  emit 0x80 + 8*g + ((V >> 3*k) & 7)
```

The three `0x0514` words are the packet tag, `0x0515` marks the word-32
boundary, and `0x80` is the first symbol. A short record therefore produces a
short word list and the rest of the 1300 bytes stays zero; `concrete_package`
returns the number of words it wrote and the sender stops there.

Worked example, SSID `CAM-LAB`, passphrase `rahasia123`:

| Quantity | Value |
|---|---|
| record | 24 bytes (`18 41 07 0a …` + `ff` + checksum) |
| groups | 8 |
| words | 70 = 3 tag + 8x8 data + 3 boundary |
| datagrams per round | 71 (70 symbols + the 500-byte trailer) |
| wall clock per round | about 570 ms (71 ms of sends + 100 ms of gap) |

## 5. What the physical test still has to settle

Written into the parity row as the open step, because this project may not
execute the vendor binary and has no camera in AP mode on this box:

1. Whether the AP-mode firmware decodes the symbol stream purely from datagram
   length, or also reads the payload window. Our transmitter sends both, so only
   a camera can tell us whether the payload is load bearing.
2. Whether the camera accepts the DVRIP write before its access point is joined
   by another client, or only after (the app joins the AP first and then logs in).
3. Whether the boundary marker repeats for records longer than 12 groups. The
   binary compares the written word index to 32 exactly once (`0xf6dbbc`,
   `subs w8, w8, #0x20`), so the implementation reproduces that literal
   behaviour instead of guessing a repeating block.
4. Timing tolerance: 1 ms per symbol and 100 ms per round are the SDK's values,
   not a measured minimum.

## 6. Physical test steps

Ten minutes, one camera, one laptop on the same Wi-Fi:

1. Reset the camera and hold the reset button until it announces the AP. Its
   access point appears as `XM-…` / `GW_AP_…` and the camera answers on
   `192.168.10.1`.
2. Join that access point from the laptop. Do not disable the laptop's own
   Wi-Fi adapter; the AP is what we are provisioning through.
3. Run the server, sign in, and ask for the plan:
   `POST /api/apmode/plan {"ssid":"<home>","password":"<key>"}`. It answers the
   record in hex, the word count, and the datagram count. Expected for a
   7-character SSID and a 10-character key: 24-byte record, 70 words,
   71 datagrams per round.
4. `POST /api/apmode/provision {"ssid":"<home>","password":"<key>","mode":"dvrip"}`.
   Expected: `ret 100` from message 1040 and `read_back true` when the camera
   echoes the SSID. This is the whole provisioning step on a camera that is
   reachable.
5. If the DVRIP path is refused, run `"mode":"broadcast"` with
   `"target":"255.255.255.255"` and 50 rounds; the call blocks for about 28 s.
   Watch the camera's LED: a successful receive usually stops the AP broadcast.
6. Bring the laptop back to the home network. The camera should join it within
   a minute, appear in the LAN search, and answer DVRIP on its new address.
7. Record in the issue: which mode worked, the `ret` value, the time from the
   last datagram to the camera joining, and the new address.

Until step 7 has been run on real hardware the parity row stays `partial`.

## 7. Server surface

| Route | Body | Answer |
|---|---|---|
| `POST /api/apmode/plan` | `{ssid, password, target?, rounds?}` | record, word count, datagram counts, no traffic |
| `POST /api/apmode/provision` | `{ssid, password, host?, target?, mode?, rounds?}` | DVRIP `ret` and read-back, or the datagram and byte totals |

`mode` is `both` (default), `dvrip` or `broadcast`. `host` defaults to
`192.168.10.1:34567`, `target` to `255.255.255.255`, `rounds` to 50. Both
routes need a signed-in session, like every other control route.

Verification commands (core only, no camera):

```
cargo test -p proview-core --lib apmode
cargo test -p proview-server --test api apmode
```

## 8. Re-checking the evidence

```
aarch64-linux-gnu-objdump -d --start-address=0xf6d828 --stop-address=0xf6e164 \
  ~/re/proview-cctv/native/arm64-v8a/libFunSDK.so      # concrete_package
aarch64-linux-gnu-objdump -d --start-address=0xf6e2f4 --stop-address=0xf6e510 \
  ~/re/proview-cctv/native/arm64-v8a/libFunSDK.so      # the socket loop
aarch64-linux-gnu-objdump -d --start-address=0xb28218 --stop-address=0xb283a8 \
  ~/re/proview-cctv/native/arm64-v8a/libFunSDK.so      # Start_Wifi
```
