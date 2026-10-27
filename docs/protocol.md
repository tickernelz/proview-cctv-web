# PROVIEW CCTV (com.proview.cctv) — protocol record

Everything below was read out of the shipped Android app (version 1.0.4,
versionCode 10403) or observed on the wire from this project's own probes.
Claims carry the artifact they came from. Nothing here is a guess presented as
fact; the "open" section says what is still unknown.

## 1. What the app is

A white-label of the Xiongmai / XMEye (JF) camera stack: a thin Java/Kotlin UI
plus one large native SDK, `libFunSDK.so` (arm64-v8a, ~34 MB, 707 JNI exports,
C++ symbols not stripped). No TUTK, no uNabto, no RTSP server of its own.

Native namespaces that matter:

| Namespace | Contents |
|---|---|
| `XMAccountAPI` | `IXMAccount` (login, device list, device tokens, DSS token), `IXMAccountData`, `IXMEncrypt` |
| `XMCloudAPI` | `IXMCloud` (device config/status, DNS), `CNetTps`, `CGWMPtl`, `SDevStatus` |
| `MNetSDK` | `CNetServerP2P`, `CNetXMP2P`, `CNetTCP`, `CNetTransport`, `CDataEncDecAES`, `CMediaChannel`, `CDssChannel`, `CGWMChannel` |
| `XMCloudStorage` | cloud recording/playback |

Third-party code linked into the same object: OpenSSL, an MQTT client, KCP
(`ikcp`), SoundTouch, FFmpeg-derived parsers, an RTSP **client**.

## 2. Cloud service map

Hosts and ports are string literals inside `libFunSDK.so`; they were read from
`.rodata` and confirmed against a live probe where marked.

| Host | Ports | Purpose |
|---|---|---|
| `access-alc.secu100.net` | 6603 / 6503 TLS | account login centre |
| `access-css.secu100.net` | 6614 / 6615 | CSS message gateway, **JSON envelope** |
| `access-pms.secu100.net` | 6602 / 6502 | platform/media service |
| `access-cfs.secu100.net` | 6609 | file service |
| `pub-cfg.secu100.net` | 8086 / 8186 | service discovery |
| `pub-status.secu100.net` | 7700-7706 / 7600-7601 | device online status |
| `pub-dss-hls.secu100.net` | 8080 / 8180 | cloud HLS restream |
| `pub-token.xmeye.net` | 80 / 443 | media token — **live, verified** |
| `jvss.xmcsrv.net`, `api.jftechws.com` | 80 / 443 | JF video/caps API |
| `d.xmeye.net`, `app.xmeye.net` | 443 | legacy REST (`.rs` family), web portal assets |

Device-side defaults baked into the SDK: TCP `34567` (DVRIP), `34571` (wakeup),
`192.168.10.1` / `172.16.10.1` as fallback LAN addresses.

The vendor runs its own browser client at `https://v2.xmeye.net`: a Vue SPA on
`https://api.xmeye.net/api`, with a player in `/player_hls_flv/` that handles
**m3u8 and flv**. Its login form demands a captcha, so it is not the path this
project automates.

## 3. Account login (mobile flow)

`IXMAccount::XM_Video_Login_XMAccount` builds:

```
POST /webservice/app/user/logInByAccount?<query>
Accept: */*
Accept-Language: zh-CN
Connection: close
Host: <host>:<port>

query: username=<email>&password=<secret>&time=<epoch-ms>&token=<token>
```

- `password` is sent as a digest, not clear text (the app formats it before the
  request; the exact digest was not needed to map the flow).
- The response carries **`Set-Cookie`**; the SDK treats a missing cookie as
  `get cookie error` and the session is cookie-based from then on.

`IXMAccount::GetPlatHttpPtl` builds the platform variant of the same call:
headers `uuid`, `appKey`, `Authorization`, `Cookie`, body
`application/x-www-form-urlencoded`, URL shape `/<group>/<cmd>/<version>/<time>/<secret>.rs`
with the version literal `v3`.

Observed against the live gateways (this project's own probes, no account
touched):

| Target | Response |
|---|---|
| `POST https://pub-token.xmeye.net/reqToken` | `{"ret":401,"msg":"uuid empty."}` then `appkey`, `tm`, `sign`, `sn`, in that order |
| `http://access-css.secu100.net:6614/webservice/app/user/logInByAccount` | `{"CssCenter":{"Header":{"ErrorNum":"400","ErrorString":"req body is not a json"}}}` |
| `http://access-pms.secu100.net:6602/...` | 400, empty body |
| `https://vmscloud.xmeye.net` | no DNS record any more |

So: the CSS gateway speaks a **JSON envelope** whose reply skeleton is
`{ "<Centre>": { "Header": { "CSeq", "ErrorNum", "ErrorString", "Version", "MessageType" }, "Body": { ... } } }`.
Reply paths seen in the binary: `CssCenter/Header/ErrorNum`,
`CssCenter/Header/ErrorString`, `CssCenter/Body/ObjInfo`,
`AlarmCenter/Header/*`, `CfgProtocol/Header/ErrorNum`,
`StatusProtocol/Header/ErrorNum`, `Shadow/Header/ErrorNum`.

## 3A. The signed cloud envelope, proven through the vendor SDK (2026-10-27)

The vendor SDK runs out of tree: `libFunSDK.so` (arm64, 33,983,560 B) inside an
aarch64 rootfs on qemu-user, chrooted, with a small stub library for the Android
and JNI symbols it imports. `re/sdk_login.sh` (ZHA-3932, not in this repository)
drives three steps and prints each result:

1. **email + password → `accessToken`.** The SDK writes `loginparams.txt`; the
   token is a JWT, HS256, `iss: rss`, `type: com`, 30-day `exp`, carrying `appKey`
   and a hashed `account` claim. Observed on every run that reached this step.
2. **Platform exchange.** `POST https://boss22-api.xmcsrv.net/account/userToken/login`
   → `{"code":2000,"msg":"success", …}` with a device token.
3. **Device list.** Login mode returns `msdlistCode`; `sdk.log` then carries
   `InitDevInfo[...]` rows.

Cost: about 65 s per login on this workstation (qemu, no KVM for this guest).

The signed request shape, read from the SDK and from the harness traffic:

```
POST https://<host>/<endpoint>/v3/<tsid>/<sig>.rs
Accept: */*
Accept-Charset: utf-8
Accept-Encoding: gzip
Content-Type: application/x-www-form-urlencoded
Content-Length: 96

body: 96 uppercase hex characters
```

- `tsid` is 20 digits: a 7-digit per-process request counter followed by
  epoch milliseconds (observed `0000003179372026023`). Section 3B derives both
  halves and the counter's wrap.
- `sig` is 32 hex characters derived per request, so a captured path cannot be
  replayed. Section 3B gives the derivation, checked against captured rows.
- The 96 hex characters are AES-128-ECB over the form body
  `account=<account>&pass=<url-encoded password>`, in uppercase hex, under a
  16-character key derived from the request timestamp and the app secret
  (section 3B). An earlier revision of this record read them as opaque key
  material; that was wrong and was corrected on 2026-10-27.
- This path carries **no** `UniqueCode` header. The `91000 Need UniqueCode`
  refusal belongs to requests that arrive unsigned.

Not in this repository: the SDK, the chroot, and the application key material the
harness passes in. None of it is redistributable, and no value of it is written
here. The harness proved the envelope; section 3B derives it and `crates/core`
now carries the client, which signs with the identity the build baked in
(`release` injects it from repository secrets through `crates/core/build.rs`)
and lets `PROVIEW_APP_UUID`, `PROVIEW_APP_KEY` and `PROVIEW_APP_SECRET`
override it per value when they are set.

## 3B. The envelope algorithm, derived from the binary (2026-10-27)

Section 3A proved the shape of the signed request. This section gives the
algorithm behind it, read out of `libFunSDK.so` and then checked two ways: it
reproduces every one of the 27 `IXMEncrypt::GetEncryptStr[...]` rows in the
harness `sdk.log`, and a Rust implementation of it logs in from this box with
no vendor binary in the loop (see 3B.6).

### 3B.1 Per-request timestamp and counter

`GetEncTimMillis` (`0xf505ec`) keeps one process-wide counter, increments it per
call, and formats

```
tsid = sprintf("%07d%lld", counter, epoch_millis)
```

which is why every captured path carries a 7-digit prefix that walks 1, 2, 3, 4
across successive calls and then a 13-digit millisecond value. When the counter
reaches `10000000` (0x989680) it wraps to 1. That bound is built from two
instructions, `mov w9, #0x9680` then `movk w9, #0x98, lsl #16`
(`0xf50630`-`0xf50634`); reading only the `movz` half gives 38528 and misses
the point by two orders of magnitude. The counter is part of the signed
material, not a nonce the server stores: a client may start anywhere.

### 3B.2 The signed string and the signature

`IXMEncrypt::SetParmeValue` (`0xf50b40`, reached only from
`FUN_XMCloundPlatformInit` at `0xbe3f80`) stores the three platform strings the
app hands in: `uuid`, `appKey`, `appSecret`, plus the scramble parameter `k`
(the app passes 4). `IXMEncrypt::GetEncryptStr` (`0xf50c38`) then, for every
request:

1. refreshes the timestamp field with 3B.1 and builds

   ```
   S = uuid + appKey + appSecret + tsid
   ```

   with one `sprintf("%s%s%s%s", ...)`; an `LD_PRELOAD` shim over
   `__vsnprintf_chk` showed that call joining exactly those four pieces, 108
   characters long for the real app constants.

2. mirrors `S` in place with the conditional swap that gives the algorithm its
   only unusual step. With `L = len(S)` and `i` from 0 to `L-1`:

   ```
   if k != 0 and (i % k) > ((L - i) % k):
       swap(S[i], S[L-1-i])
   else:
       S[i] = S[L-1-i]
   ```

   Note the `else`: it is not a no-op, it copies the mirror byte forward, which
   is why the result is not simply the reversal of `S`. `k = 4` in this build.

3. hashes the plain prefix and the reversed mirrored suffix with MD5 and prints
   lowercase hex (`OS::ToMD5` at `0xed990c`, `%02x` per byte):

   ```
   sig = md5_hex(S + reverse(mirror(S)))
   ```

### 3B.3 The body key

`XMAccountAPI::KeyFilter` (`0xf4fb4c`) builds the 16-character AES key from the
timestamp and the app secret:

```
key16 = first16( tsid[len(tsid)/2:] + appSecret )
```

When that is shorter than 16 characters it is padded on the right with the
decimal digits `0`, `1`, `2`, ... - visible in captures where the app secret
was still empty at that point, and the row read `3070132241012345`.

`XMAccountAPI::AesEncrypt` (`0xf5007c`) is AES-128-ECB with PKCS7 padding and
uppercase hex output; the key is the 16 ASCII bytes above, not a binary digest.

### 3B.4 The request and the reply

```
POST https://rs.xmeye.net/<endpoint>/v3/<tsid>/<sig>.rs
Accept: text/html, image/gif, image/jpeg, *; q=.2, */*; q=.2
Accept-Charset: utf-8
Content-Type: application/x-www-form-urlencoded
uuid: <app uuid>
appKey: <app key>
Authorization: <access token>            (device list and refresh only)

body: hex(AES-128-ECB-PKCS7("account=<account>&pass=<encoded password>", key16))
```

- Only the password is URL-encoded. `EncodeURL` (`0xad052c`) keeps
  `0-9A-Za-z / . ! - _ * ( )` and writes everything else as `%XX`; the account
  goes into the form raw, which is why an address with `@` appears verbatim.
- Endpoints seen: `loginExist` for email + password, `login` when the caller
  also supplies a second credential, `msdlistCode` for the account device list,
  `refreshToken` for the token refresh.
- A successful reply is hex AES under the same `key16`: `loginExist` gives
  `{"code":2000,"data":{"accessToken":...,"refreshToken":...}}`, `msdlistCode`
  gives `{"code":2000,"data":{"mine":[{"uuid":...,"nickname":...}]}}`, and
  `refreshToken` gives a fresh `accessToken`. Refusals arrive as plain JSON:
  `5004 SIGNATURE_ERROR`, `5005 PARAM_FAIL`, `-83000 username or password
  error`, `-91000 need UniqueCode`.
- The access token is a JWT, HS256, carrying `exp`, `iat`, `account` and
  `appKey`; the client reads `exp` from the payload instead of storing a TTL.

### 3B.5 The device token

The device token is not part of the signed envelope. `POST
https://boss22-api.xmcsrv.net/account/userToken/login` with
`{request:{authorization, appKey, appScheme:"icsee.boss.jftech.com", appVer:"1.0.2"}}`
answers `code 2000` with a platform `accessToken` and an RSA-1024 `publicKey`
(base64 DER). A sealed call then sends `Key: base64(RSA-PKCS1v1.5(publicKey,
base64(session16)))` and `{"req": base64(AES-128-ECB-PKCS7(json, session16))}`;
`/deviceToken/get` needs `deviceId` in that JSON or the server answers
`-91002 deviceId must not be blank`, which is also the proof that the seal
itself was accepted.

### 3B.6 Evidence

Where each claim comes from, so it can be re-checked:

| Claim | Artifact |
|---|---|
| `S = uuid + appKey + appSecret + tsid` | `LD_PRELOAD` shim over `__vsnprintf_chk` dumping the `%s%s%s%s` result, 2026-10-27 |
| `tsid = %07d%lld`, wrap 10000000 | `GetEncTimMillis` `0xf505ec`, bound `0x989680` from `movz`+`movk` at `0xf50630`; paths `0000003...`, `0000004...` in one process |
| mirror + MD5 order | `GetEncryptStr` `0xf50ec0`-`0xf51234`; 27 of 27 `sdk.log` rows reproduced |
| `key16` rule | `KeyFilter` `0xf4fb4c`; a row with an empty secret reads `3070132241012345` (ten timestamp digits then the digit padding), a row with the secret set keeps its first six characters after those digits |
| body plaintext | the captured 96-hex login body decrypts to `account=...&pass=...` under that key |
| device list | captured `msdlistCode` reply decrypts to `{"code":2000,"data":{"mine":[..."7f92970ddf5282743hrc"...]}}` |

Implementation: `crates/core/src/cloud/envelope.rs` (`scramble`, `signature`,
`body_key`, `seal`, `open`, `signed_request`, `EnvelopeClient`), wired into
`provider/cloud.rs`. Fixtures in `fixtures/cloud-envelope.json` use a demo
identity; the real key material is injected at build time by the `release`
workflow from repository secrets and overridable per value through
`PROVIEW_APP_UUID`/`PROVIEW_APP_KEY`/`PROVIEW_APP_SECRET`. No value and no
encrypted blob of it is stored in the sources, the workflows or the tags.

Live run from this box, `cargo run -p proview-core --example envelope_login`,
three consecutive runs on 2026-10-27: login 0.24-0.33 s, device token
0.20-0.39 s, device list 0.06-0.07 s, refresh 0.13 s, one device returned each
time (`uuid 7f92970ddf5282743hrc`, a nickname, an owner account and one
channel). The qemu harness of section 3A needs about 65 s for the same three
steps.



## 4. Media paths

Three are present in the SDK. In order of implementation cost:

1. **LAN RTSP** — the SDK embeds an RTSP client (`PLAY %s RTSP/1.0`,
   `Session:`, `Scale:`), and the devices serve RTSP on 554. No cloud needed
   while the browser host and the camera share a network.
2. **Cloud HLS (DSS)** — `CloudMediaSearchCssHls` plus `IXMAccount::GetDssCloudToken`
   and `pub-dss-hls:8080`; playlists look like
   `http://<host>:<port>/hls/<a>/<b>/<n>_<m>.m3u8`. Browser-friendly when it
   works: no P2P.
3. **P2P** — `xmnat` (vendor NAT traversal) with an `rps agent` over **UDP + KCP**
   (`conv` id, handshake, master-server failover), then the media channels on
   top. This is the expensive one; nothing here should be attempted before
   1 and 2 are exhausted.

## 4A. Cloud device status (pub-status, `/v1/status/third`)

`IXMCloud::GetDevsStatus` (`libFunSDK.so` @ `0xf54a24`) builds a JSON
envelope and POSTs it to the status cluster; the URL template is
`%s://%s:%d/%s` (strings `0x4efa47`/`0x58f47f` for the scheme, `0x58f4d7`
for the template) with the path `v1/status/third` (`0x4ae3e8`), and the
message types are `MSG_STATUS_MULTIQUERY_REQ` (`0x5028e6`) and
`MSG_STATUS_LOCALQUERY_REQ` (`0x49d43f`). The object keys read from the code
are `StatusProtocol` (`0x4946a8`), `Header` (`0x5871e0`), `Body`
(`0x519d7e`), and per entry `SerialNumber` (`0x5094ed`), `AuthCode`
(`0x575917`) and `Manufacturer` (`0x59199a`).

Request, as sent by `crates/core/src/cloud/status.rs`:

```
POST http://pub-status.secu100.net:<7700-7706>/v1/status/third
Content-Type: application/json

{"StatusProtocol":{
  "Header":{"Version":"1.0","CSeq":"1","MessageType":"MSG_STATUS_MULTIQUERY_REQ"},
  "Body":[{"SerialNumber":"<serial>","AuthCode":"<device code>"}]}}
```

Reply, measured on 2026-10-27 09:35 UTC for `7f92970ddf5282743hrc`:

```
{"StatusProtocol":{
  "Body":[{"WanIP":"103.85.122.66","SerialNumber":"7f92970ddf5282743hrc",
           "ServerPort":"8765","Status":"Online","ServerIP":"110.239.85.50","NatType":"0"}],
  "Header":{"ErrorNum":"200","ErrorString":"Success OK","Version":"1.0","CSeq":"1",
            "MessageType":"MSG_STATUS_MULTIQUERY_RSP"}}}
```

`ServerPort` arrives as a string on 7703 and on 7705; the parser accepts both
shapes. Port 7705 also carries `DeviceType`, `DevicePort` and `KcpEnable`.

### 4A.1 The ports are separate clusters, and that is the whole bug

Measured in one run, same serial, same request:

| port | answer | payload |
|---|---|---|
| 7700 | Offline | — |
| 7701 | Offline | — |
| 7702 | Offline | — |
| 7703 | **Online** | `WanIP 103.85.122.66`, `ServerIP 110.239.85.50:8765`, `NatType 0` |
| 7704 | Offline (3.0 s) | — |
| 7705 | **Online** | `WanIP 103.85.122.66`, `ServerIP 110.239.87.207:6510`, `DeviceType Camera` |
| 7706 | Offline | — |

`WanIP` equals `lastWanIp` from the session-free caps call for the same
serial, which is an independent confirmation that the online rows describe
this device. A reader that asks a single cluster therefore reports "offline"
for a camera that is online, which is exactly what the desktop app showed.
The implementation queries every port concurrently and treats the device as
online if any cluster says `Online`.

### 4A.2 What `AuthCode` is worth

The account device list (`msdlistCode`) carries per-device `css`, `dss`,
`tps`, `rps`, `p2P` and `pms` ids; `status.rs` sends the `css` id as
`AuthCode`. Measured bounds on 7703: any non-empty code is answered with the
device's real status, and an unknown serial, or a serial this cluster does not
know, is refused with `ErrorNum 400 "all dev ac is invalid"`. So the serial and
the cluster carry the identity, and the code is not a secret that gates the
answer. The refusal is still used as a signal: when no port returns a
`Status` the app reports "cloud status unknown" with the vendor's reason
instead of claiming offline.

### 4A.3 What the app exposes

`GET /api/devices` answers `online`, `online_status` (the vendor string),
`online_checked_at` (epoch ms) and `online_detail` (cluster port, WAN IP and
relay endpoint, or the reason the status could not be read). The live screen
blocks on a known offline status only; an unknown status lets the stream be
attempted and shows the stream's own error.

## 4B. What the cloud can and cannot stream (measured 2026-10-27)

Three mechanisms exist in the SDK; only one of them delivers **live** video, and
the other two were probed against the live services today.

### 4B.1 CSS HLS is cloud-storage playback, not live

`CloudMediaSearchCssHls` (`0xaff748`) builds a `CssCenter` envelope:

```
{"CssCenter":{
  "Header":{"Version":"1.0","MessageType":"MSG_GET_CSS_HLS_REQ"},
  "Body":{"SerialNumber":…,"AuthCode":…,"UserId":…,"MAUserId":…,
          "StartTime":"YYYY-MM-DD HH:MM:SS","StopTime":…,"Channel":"1"}}}
```

The keys were read from the binary (`0x49274f` CssCenter, `0x4cef8c`
MSG_GET_CSS_HLS_REQ, `0x5094ed` SerialNumber, `0x575917` AuthCode, `0x55c9ab`
UserId, `0x49b977` MAUserId, `0x57e4dc` StartTime, `0x4bda55` StopTime,
`0x56d469` Channel).

Posted to `http://access-css.secu100.net:6614/` with the account's device row
and the `userId` from `msdlistCode`, the gateway answers:

```
{"CssCenter":{"Header":{"ErrorNum":"403","ErrorString":"Dev Not Open Cloud Storage", …}}}
```

`Body` must be an object; an array answers `402 invalid sn`, and `Sn` instead of
`SerialNumber` is also `402`. So this endpoint is the **cloud-storage search**
(the recording list) and this camera has no cloud storage plan. It cannot serve
live.

### 4B.2 The DSS HLS host wants a signed playlist

`http://pub-dss-hls.secu100.net:8080/hls/<sn>/<a>/<b>/<name>.m3u8` answers
`403` for every unsigned shape tried and `400` for the shorter ones; the host is
alive on 8080 and 8180. The URL template in the SDK is
`%s://%s:%d/hls/%s/%s/%s/%s.m3u8` with a companion
`%s://%s:%d/hls/stop/%s/%s/%s/%d`, so the query string carries the signature.
Nothing in the account session produces that signature yet.

### 4B.3 Live is the P2P media channel

The app's live player does not use a URL. `MediaPlayByURL`/`MediaPlayM3u8ByUrl`
are only reached from the recording players (`ua/c.smali`, `va/a.smali`, both
log lines say `RecordPlayer`), while live uses `FunSDK.MediaPlay` after a device
connection. The transport behind that call is the SDK's own stack:
`CNetServerP2P`, `CNetXMP2P::NatConnect`, `CNetTransport`, `CDataEncDecAES`
and `CMediaChannel`, with `rps` over UDP+KCP.

The status reply of section 4A gives the endpoints that stack uses: 7703 says
`ServerIP 110.239.85.50:8765`, 7705 says `110.239.87.207:6510` with
`DevicePort 34567`. Those are the relay and device ports the app dials.

Next step for this milestone is to drive the SDK harness (`re/sdk_login.sh`
machinery, `harness12.c`) through `MediaPlay` with a packet capture on the
host, so the wire format is recorded once and the Rust client can be checked
against it byte for byte.

### 4B.4 The JF cloud device page is the camera's ActiveX web UI

`cloud.jufenginfo.com` does serve the device over the cloud: a device login
(`cloud_device_macLogin_main`, status 1200) frames `/cloud/video/index.jsp`,
and that page loads 105 KB of the camera's own web client ("NETSuveillance
WEB"). Its scripts (`config.js`, `m.js`, `English.js`, fetched with the same
session) reference only `http://xmsecu.com:8080/ocx/New_Active.exe` and
`NewActive.exe`: the live view is a Windows ActiveX control. The portal is an
HTTP proxy to the camera's embedded web server, not a video relay, so it cannot
stream to a browser and is not a fourth media path.

## 4C. The P2P wire, as the phone and the SDK actually speak it (2026-10-27)

Captured from the owner's phone with PCAPdroid while the real app played live,
and cross-checked with a host capture of the SDK harness in the qemu chroot.
Two captures, two independent clients, same shape.

### 4C.1 Relay discovery is one UDP exchange on 8765

The client sends a 4-byte frame to a relay on UDP `8765`:

```
12 20 f0 05
```

and gets back a list:

```
12 20 f1 05 | 03 00 00 00 | "159.138.100.133\0" 55 00 00 00
                          | "159.138.101.123\0" 45 00 00 00
                          | "110.239.71.224\0"  00 00 00 00
```

`0x1220` is the magic, `0xf005` asks for the relay set, `0xf105` answers with a
count and one NUL-terminated IPv4 per entry plus a trailing u32. The same
exchange appears in the SDK harness capture from this box, so the sequence is
not phone-specific.

### 4C.2 The relay tunnel is TCP 6611 with a base64 handshake

The phone's live video is one TCP flow: **9.5 MB downstream in 17 s to
`162.128.66.154:6611`** (~4.5 Mbit/s), no TLS. The first client payload is
base64 text:

```
aaaaaaaa-1a1b397d2:c_6kjkfw65cvzl39rjrno8yc0j:7f92970ddf5282743hrc:cy7cimmp5
```

i.e. `<rps id>:<client session id>:<device serial>:<client token>`, where the
first field is the `rps` value the account device list already carries. The
relay answers base64 JSON plus a 4-byte trailer:

```
{"ErrorNum":"200","DestIP":"103.85.122.66","MTU":"1400"}XXEE
```

`DestIP` is the device's WAN address, the same `103.85.122.66` the status
clusters and the caps call report.

### 4C.3 Inside the tunnel it is plain DVRIP

After the handshake the flow carries DVRIP frames, starting with
`ff 01 00 00` and the usual session/sequence/channel/type/length fields. Two
examples from the capture, payload text included:

```
ff010000 9f860100 10000000 6300 85000000
  {"Name":"OPMonitor","OPMonitor":{"Action":"Claim","Parameter":{"Channel":0,...
ff010000 2f000000 18000000 0000 e903 80000000
  { "AliveInterval" : 30, "ChannelNum" : 1, "DeviceType " : "IPC", "ExtraChannel" : 0
```

`OPMonitor` is the SDK's device-monitor claim and the second frame is the
device's own configuration answer, both familiar shapes from the DVRIP work in
`crates/core/src/dvrip`. That is the opening for this project: the media
channel is DVRIP over the relay tunnel, so the existing DVRIP client can carry
live once the relay handshake and the framing differences are implemented.

### 4C.4 What that means for the implementation

1. `udp 8765` → `0x1220/f005` for the relay set (cache it; it changes).
2. `tcp <relay>:6611` → base64 `<rps>:<client>:<serial>:<token>` → base64 JSON
   with `ErrorNum 200` and `MTU`.
3. DVRIP over the tunnel: login with the device credentials, then the live
   request the SDK sends (`OPMonitor` claim plus media play), and the frames
   arrive in the same envelope the LAN path already parses.
4. The `rps` id and the device credentials come from `msdlistCode`, so no new
   account call is needed.

## 5. Open items

1. ~~The CSS request envelope for account login~~ — answered for the SDK path in
   section 3A (signed `/v3/<tsid>/<sig>.rs` envelope, proven in the vendor SDK);
   what is still open is a client that can produce it without the vendor library,
   and the v2 portal's own captcha/login.
2. `sign` / `sn` derivation for `/reqToken`.
3. The discovery call that maps an application id to the live host:port set
   (`GetServerDSNAndPort`, `pub-cfg`).
4. Answered: RTSP 554 with Digest auth; paths and handshake are in section 8.
5. P2P media channel details.

## 6. DVRIP transport as implemented here

The device-facing transport in `crates/core/src/dvrip/` follows the framing that
the SDK's `CNetTCP`/`CDevProtocol` classes speak: a fixed 20-byte header
(`0xFF`, then session, sequence, channel, end-flag, message id, body length, all
little-endian) followed by a JSON body terminated with `\n\0` on the wire. The
client trims that tail on the way out of the socket before parsing.

Login is a single request. `PassWord` carries the vendor's "sofia" hash: MD5 of
the password, then every pair of adjacent digest bytes summed modulo 62 and
mapped onto `0-9A-Za-z`, which yields eight characters. That is not a
reconstruction: the same function ships in the device's own web UI as `MD5_8()`
(`http://<ip>/js/main.js`), and it was confirmed on the wire afterwards.

Observed on the IPC at `192.168.0.100` (firmware
`V5.01.R02.G51A07F3.10010.346732.S.ONVIF 21.06`):

| Request | Reply |
|---|---|
| login, `PassWord` = sofia hash | `Ret: 100`, `SessionID "0x00000017"`, `DeviceType "IPC "`, `ChannelNum 1`, `AliveInterval 30`, plus an `AdminToken` |
| login, `PassWord` = `""` | `Ret: 203` (password is incorrect); no `Realm` challenge is offered |
| login, `PassWord` = 32-character MD5 hex | `Ret: 203` |

So the `ret 203` this project used to report was its own encoding, not the
credentials: RTSP Digest and DVRIP now accept the same password. The candidate
walk (plain MD5, double MD5, realm digest) is gone; the client sends the sofia
hash once and raises a typed error otherwise.

Message ids verified against this device: `1000` login (reply `1001`), `1001`
logout, `1020` SystemInfo, `1040` config set, `1042` config get, `1360`
abilities, `1452` OPTimeQuery, `1560` OPSNAP (binary JPEG reply on the same
socket). Config names that exist on this firmware and are used by the control
routes: `SystemFunction` (capability flags), `Camera` (its `WhiteLight` block and
`Param[0].PictureFlip`/`PictureMirror`), `Record` (`RecordMode`), `SystemInfo`.

A burst of rejected logins makes the device refuse **every** TCP port (80, 554,
34567) for about a minute while ICMP still answers; it recovers by itself. That
is why the client never loops over hash candidates.

## 7. Method

`apktool` (manifest, resources), `jadx` (DEX), `readelf`/symbol table plus a
small ADRP/ADD cross-reference scanner over the aarch64 `.text` to bind string
literals to the functions that use them, plus plain HTTP probes for the live
behaviour. The APK, the `.so` files, the extracted application credentials and
any capture stay outside this repository.

## 8. RTSP transport as implemented here

The IPC on the owner's LAN (192.168.0.100) answers RTSP on 554 with
`Server: H264DVR 1.0` (the OEM stack name, not proof of a DVR) and Digest auth,
realm taken from the `WWW-Authenticate` challenge. `crates/core/src/rtsp.rs`
speaks the session by hand:

1. `DESCRIBE` without credentials → `401` plus the challenge; the same request
   is replayed with the RFC 2069-style digest (`HA1 = md5(user:realm:password)`,
   `HA2 = md5(method:uri)`, no `qop`).
2. The 200 body is SDP parsed into tracks. Parameter sets arrive **base64** in
   `a=fmtp`: `sprop-vps`, `sprop-sps`, `sprop-pps`. The parser also accepts a
   hex encoding because the same OEM ships both.
3. `SETUP` uses `Transport: RTP/AVP/TCP;unicast;interleaved=0-1`. The device
   may answer with a different pair (`interleaved=2-3` was observed); the
   assigned pair is read from the response and used, never assumed.
4. `PLAY`, then media arrives as interleaved frames: `0x24`, channel byte,
   length (big-endian u16), RTP packet.
5. `TEARDOWN` when the viewer leaves.

Stream selection uses the OEM path instead of a second session: `stream=0` is
the main stream (2304×2592 H.265 on this camera, 25 fps declared, ~12 fps
delivered) and `stream=1` the sub stream (640×720). Both pull fine with the
same Digest credentials.

### RTP → HEVC

`crates/core/src/media/rtp.rs` decodes the fixed header, CSRC list, extension
header and padding. `crates/core/src/media/hevc.rs` depacketizes RFC 7798:
single NAL units pass through, type 48 (AP) is split on its 16-bit length
prefixes, type 49 (FU) is reassembled, type 50 (PACI) is skipped.

The FU header carries the first byte of the NAL header (`F` and `Type`), and the
**second byte must be taken from the FU payload header** rather than from the
fragment data. Dropping it produced a stream that ffprobe read (parameter sets
intact, resolution correct) but could not decode: every access unit lost one
byte. Caught by pulling six seconds from the real camera and running
`ffmpeg -i capture.h265 -f null -`: 72 of 72 frames decoded after the fix, zero
before.

### HEVC → browser

Browsers cannot be assumed to decode HEVC (WebKitGTK usually cannot, Windows
depends on the installed codec). The server transcodes once per stream: NAL
units go into `ffmpeg` as Annex-B (`-f hevc` in, libx264 baseline out,
`empty_moov+frag_keyframe`), and the fMP4 on stdout is split on box boundaries
into one init segment plus one fragment per keyframe. Both are broadcast over
WebSocket; the hub keeps the init and the newest fragment so a late viewer
starts on a complete keyframe. Numbers live in `docs/performance.md`.

`-fflags nobuffer` on the input demuxer was measured to discard frames on this
stream (0 decodable frames instead of 72), so it is deliberately absent.

## 9. Device control transport (ONVIF)

`POST http://192.168.0.100:8899/onvif/device_service` speaks SOAP
(`Server: hsoap/2.8`) and serves device, media, imaging and PTZ on the same
path. Port 80 is the web UI: it answers `200` HTML for every path, so a 200
there is not evidence of an ONVIF surface. UDP 3702 (WS-Discovery) does not
answer. None of the calls below needed WS-Security.

Measured against the IPC:

| Call | Result |
|---|---|
| `GetDeviceInformation` | `H264` / `XM535_X6E-WEQ_8M` / `V5.01.R02.G51A07F3.10010.346732.S.ONVIF 21.06` |
| `GetProfiles` | one profile `000`, video source `V_SRC_000`, encoder `000` |
| `GetNodes` | `NODE_000`, 128 presets, `HomeSupported=true`, auxiliaries `Wiper start/stop` |
| `GetPresets` | tokens `0`, `1`, `2` |
| `ContinuousMove` with `Timeout` | `200`. `GotoPreset 1` moves the view 28.7 mean-luma points away from the home position, while presets `0` and `2` differ by 1.65 (same position), measured on 320x360 grey frames |
| `GetVideoEncoderConfiguration 000` | H264 3072x2048, 12 fps, 1536 kbps, GOP 36 |
| `GetImagingSettings` | `200` (brightness 50, `IrCutFilter OFF`, exposure `AUTO`) |
| `GetNetworkInterfaces` | `eth0` 192.168.1.10, `eth3` 192.168.0.100 |
| `GetStatus` | `ter:ActionNotSupported`: the firmware reports no position |
| `RelativeMove` | **blocked in code**: on this firmware it stops every device service (80/554/8899/34567) for 35-45 s |

The code follows from those results: every movement is a `ContinuousMove` with
an explicit `Timeout` so the device stops itself, `RelativeMove` is refused
before a packet leaves, and the logical position lives in the application and is
restored with `GotoPreset`.
Right after a `ContinuousMove` the endpoint can refuse a request for a second or
two; only the idempotent `GotoPreset` is retried (three attempts, 1.5 s apart),
never a movement, so a retry can never move the camera twice. Measured through
the shipped route: home vs moved = 20.5 mean-luma points on a 0.4 noise floor,
and `GotoPreset 0` brings the view back to 5.6, i.e. closer to home than to the
moved position.

Picture flip is the opposite case and stays `blocked`: `Camera.Param[0]` carried
`PictureFlip`/`PictureMirror` (a write of `0x00000001` was acknowledged and read
back, then restored), but the device now answers the same `Camera` read with an
empty `Param` list, so the current state cannot be read. The control route and
the capability row both report that reason instead of a value. PTZ is bound to the lower lens; the upper lens is
reported `blocked` per lens instead of hidden. DVRIP (`34567`) stays the
fallback for movement and the only path for settings, snapshot and record.

Two-way audio stays blocked with a recorded offer rather than a guess: the RTSP
SDP carries `m=audio 0 RTP/AVP 8` (`PCMA/8000`, `trackID=4`) but no backchannel,
and the ONVIF device service exposes no audio output. DVRIP reports
`SupportTwoWayVoiceTalk=true`, so the missing piece is a capture path in the
server, not the device API.

Multi-channel: the device reports one media profile and `ChannelNum 1`; the dual
lens arrives as one frame, so a two-viewport grid needs cropping, not a second
channel.

## 10. Cloud account: the v2 portal, as implemented here

`crates/core/src/cloud/` implements the account cloud of the vendor's own
browser client (`https://v2.xmeye.net`, a Vue SPA over
`https://api.xmeye.net/api`), and the server mounts it at `/api/cloud/*`:
`portal`, `captcha`, `session`, `logout`, `profile`, `devices`, and the
session-free `device-caps` of section 10B.

### 10.1 Endpoints

| Call | Request | Reply |
|---|---|---|
| captcha | `POST /api/transfer/image/v1/`, empty JSON body | `{code:200, data:{image:"<base64 JPEG>", codeToken:"<uuid>"}}` |
| login | `POST /api/transfer/login/v1`, `{uname, upass, verCode, pkg?}`, header `Code-Token: <codeToken>` | `{code:2000, accessToken:"…"}` |
| profile | `POST /api/transfer/userinfo2/v1`, `{Authorization:"<token>"}` | `data:{username, phone, email, …}` |
| devices | `POST /api/transfer/mdlist/v1`, `{Authorization:"<token>"}`, header `Access-Token: <token>` | `data:{data:[{uuid, nickname, username, …}]}` |
| status | `POST /api/transfer/devstatus/v1`, body `[{uuid}, …]`, header `Access-Token: <token>` | `data:[{uuid, status:"online" or "notfound", wakeUpStatus}]` |
| caps | `POST /api/caps/status`, `{ver:2, sn:[uuid], caps:["xmc.css","net.cellular","net.dat"]}` | `data:{capsData:[{sn, cssNetVo, celVo}]}` |

`pkg` is read by the SPA from its own URL (`?pkg=…`) and omitted when absent;
it is not a proven tenant selector. Every authenticated call carries the token
**twice**: inside the body as `Authorization` (an odd place, but that is what the
SPA sends) and in the `Access-Token` header.

### 10.2 The field digest

Both `uname` and `upass` are AES, not clear text. The encoder is webpack module
`c71e`, shipped in `chunk-0c782707.ebc888f7.js`:

- key: `d86d7bab3d6ac01ad9dc6a897652f2d2`, parsed with `CryptoJS.enc.Hex.parse`
  (16 bytes, so AES-128);
- mode `ECB`, padding `Pkcs7`, output `CipherParams.toString()` = standard
  base64 of the ciphertext, with no OpenSSL `Salted__` prefix;
- `verCode` stays clear text; the account name travels digested too.

Cross-checked byte for byte against CryptoJS 4 in node and `openssl enc
-aes-128-ecb -nosalt -base64` for the same input, and implemented in
`crates/core/src/cloud/digest.rs` with a reference vector in its tests.

### 10.3 Session and tokens

A successful login returns `accessToken` at the **top level** of the reply, not
inside `data`. There is no refresh token and no server-side logout route: the
SPA's "log out" only clears `localStorage`/`sessionStorage`. Because a second
login needs a fresh captcha, an expired token cannot be renewed silently; this
implementation drops the server-side session and asks the user to sign in
again (`CloudError::Expired`, HTTP 401).

The server keeps the token in its own process, keyed by an opaque session id in
an `HttpOnly`, `SameSite=Strict` cookie. The account password is never stored:
it is digested per login call and dropped.

### 10.4 Reply codes seen

| Code | Meaning |
|---|---|
| `2000` | success (login, device list, profile) |
| `200` | success (captcha image, `caps/status`) |
| `-1` | captcha wrong |
| `-2` | captcha expired |
| `4010` | captcha error |
| `4048` | too many wrong captchas |
| `4050` | requests too frequent |
| `4000` | username or password wrong |
| `4024` | email not registered |
| `4025` | username not registered |
| `4017` | username format rejected (checked on the decrypted value) |
| `-4004` | `Access-Token` expired or missing |

Codes are mapped to readable text in `cloud::types::user_message`; `-4004`
becomes a session-expired error so the UI can re-prompt for a captcha.

### 10.5 Live evidence (2026-10-26, this project's own probes)

The digest is proven against the live service, not just against the bundle:

| Probe | Reply |
|---|---|
| `usercheck/v1 {name: AES("zznobody9")}` | `4025 REG_USER_NOT_EXIST` |
| `usercheckMail/v1 {mail: AES(<configured account>)}` | `4024 REG_MAIL_NOT_EXIST` |
| `usercheck/v1 {name: <clear text>}` | HTTP 500 `-4004` (undecryptable) |
| `login/v1` with a wrong captcha | `-1` |
| `login/v1` with a valid captcha for the configured account | `4000` |
| `POST /api/cloud/session` on this server, solved captcha, real account | HTTP 400 `email or password is wrong` (vendor `4000`) |

The last row is the whole pipeline: the server digested the configured
account's fields, the captcha was accepted (otherwise the vendor answers `-1`
or `-2`), and the account itself was refused. An undecryptable field would
have produced `-4004` instead, so the digest really is what the service read.

Two conclusions. First, the service decrypts the field and validates the
decrypted value (a format-invalid plaintext returns `4017`), so the key, mode,
padding and encoding are correct. Second, the answers above are about the
**request as we sent it**: an email as `uname` on `api.xmeye.net` is answered
`4024`/`4000`, i.e. this service does not hold that email as an account.

What that does **not** prove is that the owner's account is absent from the
vendor cloud. The app shows an account id (`67e…8302`) next to a filled email,
so the login identifier is probably the id, not the address. Measured bounds:

| Probe | Reply | Reading |
|---|---|---|
| `usercheck/v1 {name: AES("67e…")}` (id-shaped, not the real id) | `4025 REG_USER_NOT_EXIST` | the portal accepts that shape as a username, so the id hypothesis is testable here |
| `usercheck/v1 {name: AES("a@b.c")}` | `4017 REG_USER_FORMAT` | an address is not a valid `uname` on this endpoint at all |
| `usercheckMail/v1 {mail: AES(<email>)}` | `4024` | the email is not bound to a portal account |

So `4024` for the address is consistent with "the address is not the login
identifier", not only with "the account is elsewhere". Whoever fills in the
real id can settle it with one `usercheck` call.

The mobile path is a different protocol on different hosts, now measured: the
SDK's `logInByAccount` target comes from `GetServerDSNAndPort`, an ALC/CSS
gateway. On this network `access-alc.secu100.net` answers on 6602
(`AlarmCenter`), 6502 (TLS, `AlarmCenter`), 6614 and 6615 (TLS)
(`CssCenter`); 6603/6503 are closed, and every path on those gateways returns
the same JSON-envelope error, so the URL path carries no meaning there — the
message does. The transfer API is not served there: `api.xmeye.net` has
`/api/transfer/*` and no `/webservice/*` (404 from nginx), and the gateways
have the envelope and no `/api/transfer/*`.

### 10.6 What the client does not do

- It does not solve captchas; `GET /api/cloud/captcha` returns the image and its
  token to the caller, and the code is typed by a human.
- It does not call `caps/status`; device capabilities are mapped only from the
  fields of `mdlist/v1` (`channel`, `channelNum`, otherwise one channel).


## 10A. The app's own cloud is REST on api.jftechws.com

Captured from the owner's phone through mitmproxy after the app was patched to
trust a user CA (Wira, 2026-10-26). This is the app's account/feature API, and it
is **not** the portal of section 10 and **not** the `CssCenter` gateway of
section 3:

```
POST https://api.jftechws.com/privacy/query
  uuid: <the APP_UUID from the manifest>
  appKey: <the APP_KEY from the manifest>
  versionName: 1.0.4
  Accept-Language: in
  Authorization: (empty before login)
  Content-Type: application/json; charset=UTF-8
  User-Agent: okhttp/3.12.0
  body: {"accessKey":"…","lan":"in","pkg":"com.proview.cctv"}
```

The base URL is a literal in the APK (`com/xworld/utils/PrivacyUtils.java`:
`https://api.jftechws.com`), it speaks plain REST/JSON, and `Authorization` is
the slot that fills in after login.

Route behaviour from this network (headers as above, empty body where not
noted):

| Call | Reply |
|---|---|
| `POST /privacy/query` with `{accessKey, lan, pkg}` | `{"code":5004,"msg":"Param Error","data":null}` |
| `POST /login`, `/user/login`, `/account/login`, `/app/login`, `/user/query`, `/device/list`, `/user/register` | `{"error_msg":"404 Route Not Found"}` |

The 404 body is the framework's, so a route exists only when the reply carries
`code`/`msg`/`data`. That makes the surface enumerable without credentials, but
only along the paths the client actually calls — the login path is not among
the Retrofit interfaces in the APK (they cover privacy, promotion, cloud drive,
customer service and the legacy `…/{version}/{timeMillis}/{secret}.rs` family),
so it is either native or built at runtime.

Two hardening facts that shape any further capture attempt:

1. A repackaged build (the `apk-mitm` step needed for user-CA trust) is
   redirected to Google Play on launch, so the app checks its own signature.
2. The original build was restored, and targetSdk 35 means it does not trust a
   user CA either. Capturing its TLS therefore needs a pcap (host and port
   only), a rooted device, or a Frida hook — not another certificate install.

The account login is **not** part of this REST surface. `libFunSDK.so` carries
the login path itself — `/webservice/app/user/logInByAccount` with query
`username=%s&password=%s&time=%lld&token=%s` and a `Set-Cookie` session
(section 3) — and `IXMAccount::XM_Video_Login_XMAccount` at `0xf3fc2c` builds it,
AES-encrypting both fields first. The host for that call comes from
`GetServerDSNAndPort`, i.e. one of the ALC gateways, not from
`api.jftechws.com`.

So the surface splits cleanly:

| Surface | Host | What it carries |
|---|---|---|
| portal (section 10) | `api.xmeye.net` | web client login, device list, captcha |
| app REST (this section) | `api.jftechws.com` | privacy, promotion, cloud drive, binding (`/listbind`, `/createbind`), caps |
| app account login (section 3) | ALC host from discovery | `/webservice/app/user/logInByAccount`, AES query, cookie session |

A capture that only shows `api.jftechws.com` therefore says nothing about the
login itself; the login is the native call, and reading it needs either a pcap
(the gateway answers plain HTTP on some ports, so the query may be in cleartext)
or a Frida hook.


## 10B. A cloud session without the account: the JF portal device login

`cloud.jufenginfo.com` is the classic JF cloud portal. It accepts a **device**
login, so a cloud session exists without any account credential:

| Call | Result |
|---|---|
| `GET /verification_produce?_=2` | JPEG captcha + `JSESSIONID` |
| `POST /cloud_device_macLogin_main` with `deviceMy.mac`, `deviceMy.username`, `deviceMy.password`, `verificationCode2` | `{"status":1200,"tips":"success:"}`, session rotated and a `macCookie` issued |
| `GET /macLogin` | frames `/cloud/video/index.jsp?mac=…&username=…&password=…` |
| `GET` that frame | `200`, 105,009 bytes, `<title>NETSuveillance WEB</title>` |

Reproduced on 2026-10-26 from this box with the device serial
`7f92970ddf5282743hrc` and the credentials in `secrets/device.env`: the login
returned `1200`, the cookie jar held `JSESSIONID` + `macCookie`, and the framed
page came back with the byte count and title above. Device username and password
are encrypted by the portal page before the call; the same values from the
working LAN session are accepted.

What this is and is not:

- It is a **cloud** path to the device UI, reachable from anywhere the portal is
  reachable, and it needs no account, no captcha solving beyond the device step,
  and no P2P.
- It is **device-scoped**, not account-scoped: it does not enumerate the
  account's devices, so it cannot replace `/transfer/mdlist/v1`.
- Sessions are per family. The portal rotates `JSESSIONID` on every
  session-creating call, so a device jar and an account jar must not be shared.

### Correction to section 10.1: never send `pkg`

On `/api/transfer/login/v1`, adding a `pkg` field to the body makes the server
answer `5005 wrong param` for **any** credentials, correct or not. Without
`pkg` the answers are the meaningful ones (`2000` success, `4000` wrong
credentials). The Rust client does not send `pkg`, which is why its answers were
`4000` rather than `5005`. Treat any `5005` as "the request shape is wrong", not
as "the account is wrong".

Related rate limit, measured: around ten `usercheck` calls in a few minutes make
every shape answer `5005` as well, so burst probing erases information.


## 10C. The iOS app's cloud surface, and the one call it makes without a session

Captured on 2026-10-27 from the owner's iPhone through Proxyman with SSL proxying
on every domain (290 entries, 174 decrypted), written up in full in
`cloud-app-surface.md`. The app (`com.szyarsor.proviewcctv` 1.0.2) never touches
`api.xmeye.net`; it speaks to `api.jftechws.com`, `caps-as.jftechws.com`,
`as-ams.jftechws.com`, `boss22-api.xmcsrv.net`, `caps.xmcsrv.net`,
`app-support.xmcsrv.net`, `upgrade.secu100.net` and `promotion.xmeye.net`.

Three facts that matter for this project:

- The login the app does complete is
  `POST boss22-api.xmcsrv.net/account/userToken/login` with
  `{request:{authorization:<JWT>, appKey, appScheme, appVer}}`; the reply echoes
  the same HS256 JWT (`iss rss`, `type com`, `account <account id>`) as
  `accessToken`, plus an RSA-1024 `publicKey`. Where that JWT is minted stays
  unseen: `pub-dscn.secu100.net` was the only vendor host still tunnelled while
  every other host decrypted, i.e. certificate pinning, and the token's `iat`
  falls inside that host's window.
- The device list is `POST as-ams.jftechws.com/mdsharemylist/v1/<tsid>/<sig>.rs`
  with `shareUuid=`; each row's `powers.devInfo` is encrypted and the path
  signature is per request, so nothing there can be replayed.
- `POST caps.xmcsrv.net/api/asMsgQ` with
  `{"method":"reqCaps","body":{"caps":[…],"sn":<serial>}}` answers model,
  hardware id, firmware, `lastWanIp` and the capability block **without any
  session**. That is the only cloud call this project ships so far:
  `GET /api/cloud/device-caps?sn=<serial>` over `cloud::app::CapsClient`. An empty
  serial answers 400; a serial the cloud does not know answers `ret 200` with
  blank `model`/`pid`, which the route reports as 404.

Reaching the pinned host, and with it the minting request and the device list,
needs a hooked or jailbroken client, or static work on the Mach-O, not another
proxy export.

## 11. Live passthrough transport (stream 2)

A second live transport skips the transcode: `/ws/live?...&codec=hevc` forwards
the camera's HEVC access units straight to a WebCodecs client. The socket opens
with one JSON text frame
`{"type":"hello","mode":"passthrough","codec":"hvc1.1.6.L63","width":2304,"height":2592,"audio":"PCMA","audio_clock":8000}`
followed by binary frames:

| first byte | payload |
|---|---|
| `0x01` | HEVCDecoderConfigurationRecord (`hvcC`), for `VideoDecoder.configure({codec, description})` |
| `0x02` | keyframe access unit: `u64` pts in µs (big endian), then `u32` length + NAL, repeated |
| `0x03` | delta access unit, same payload layout |

A viewer joins at the next IDR and is resynchronised at the next IDR when it
falls behind; an RTP sequence gap drops the partial fragment and only a NAL
with its first-slice flag at an access-unit boundary restores the stream. The
codec string comes from the SPS `profile_tier_level` and the SPS payload is
de-escaped (`00 00 03` → `00 00`) before it is parsed, otherwise the camera's
SPS yields a zero size. Every keyframe access unit repeats VPS, SPS and PPS in
band. The MSE transport keeps the raw fMP4 framing: init segment first, then
fragments with no header bytes.

`GET /api/live/stats` (session required) reports one row per active hub key:
transport, subscribers, publishers, packets, keyframes, bytes, audio packets,
drops, codecs and dimensions. The measurement tool reads this endpoint before
sampling and refuses a passthrough sample while a transcode publisher for the
same camera is still alive, because the two transports would otherwise share
the CPU figure.
