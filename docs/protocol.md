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

### 4C.5 The device address query, and what a replay does not do

Right before the media connect the phone asks the relay set for the device's
current address on UDP `8777`:

```
15 20 00 b0 | "7f92970ddf5282743hrc\0"
15 20 01 b0 | 01 00 00 00 | serial\0 | zeros | "110.239.85.50\0" | zeros |
              3d 22 00 00 | 00 00 00 00 | 02 00 00 00
```

`0x1520` is the magic here, `0x00b0` asks by serial and `0x01b0` answers with
the device's relay `110.239.85.50` and port `0x223d` = 8765.

The 8777 query also answers this box, so it is not phone-specific:

```
send 152000b0 + "7f92970ddf5282743hrc\0"  -> 140 bytes
recv 152001b0 01000000 "7f92970ddf5282743hrc\0" ... "110.239.85.50\0" ... 3d220000 00000000 02000000
```

The reply names the relay the device sits on right now (`110.239.85.50`, then
`0x223d` = 8765, then `2`), but the phone's media flow went to a different
host on 6611, so the mapping from this reply to the media endpoint is still
open: the trailing values are the candidate fields.

Replaying the phone's exact 136-byte media handshake from this box against
`162.128.66.154:6611` - and the same text with CRLF or base64-decoded - gets
no reply at all, while the TCP connect itself succeeds. So the relay only
answers a client it has just associated through the query path. The next step
is to drive 4C.1/4C.5 from one socket lifetime and then handshake, which is
what the SDK does inside a single `DevLogin`.

### 4C.6 The relay address comes from the RPS agent, and live plays from this box

The SDK harness (`re/qemu/harness12.c`, mode `live`) plays live from this box:
one run pulled **12 MB in 90 s** from `162.128.66.64:6611` (host capture of the
run: TCP conversation `172.23.199.41:48500 <-> 162.128.66.64:6611`, 10845
inbound segments, 13 MB total).

The SDK log names the whole chain, file and line included:

```
agent_session: session_id = aporjw9z96js0jin0qludar0yird3g8e,
               access_ip = 110.239.87.207, access_port = 6510,
               uuid = 7f92970ddf5282743hrc, dest_port = 34567, svr_type = RpsAV
agent create socket (110.239.87.207, 6510)
agent_session, set_agent_server, get tcp port addr: 162.128.66.64:6611
agent create socket (162.128.66.64, 6611)
```

So the media relay is **assigned per session by the RPS access server**, dialled
on the `ServerIP:ServerPort` the status cluster already reports: cluster 7705
says `110.239.87.207` with `DevicePort 34567`, and the access port is 6510.
That is why replaying the phone's handshake against `162.128.66.154:6611` was
never answered (4C.5): the tunnel only serves a client the RPS has just
registered, and 8777/7999 only hand out the 8765 discovery relays.

Our own client's handshake on the assigned agent, straight from that capture:

```
-> base64 aaaaaaaa-1a1b2f27d:c_v4w9y...:7f92970ddf5282743hrc:<token>
<- base64 {"ErrorNum":"200","DestIP":"103.85.122.66","MTU":"1400"} + "XXEE"
```

Immediately after it: `ff010000 ... {"Name":"OPMonitor","OPMonitor":{"Action":"Claim",...}}`,
the device's `{"AliveInterval":30,"ChannelNum":1,"DeviceType ":"IPC",...}` answer,
then the media stream in the same DVRIP envelope the LAN path already parses.

What is still missing is the body of the 6510 conversation: it is TLS 1.3 and
the SDK links OpenSSL statically, so an `LD_PRELOAD` of `SSL_read`/`SSL_write`
does not interpose (measured: zero hook hits across a full live run). The next
measurement is an inline hook at the `SSL_write` symbol address inside the
harness process, which returns both bodies unchanged.

### 4C.7 The RPS access server is HTTP inside TLS, and answers AgentProtocol JSON

Captured in plaintext from the SDK harness with an inline hook at the `SSL_write`
symbol address (`LD_PRELOAD` cannot interpose: the SDK links OpenSSL statically,
measured as zero hook hits). The RPS access endpoint takes an HTTP request on the
raw port the status cluster reports, `POST /` with `Host: 110.239.87.207`:

```json
{"AgentProtocol":{"Body":{"Authcode":"aaaaaaaa-1a1b2f27d","ClientToken":"c_1yy07itliajqv91mz6249eet","DestPort":"34567","Encrypt":"1","ProtocolMode":"tcp","SerialNumber":"7f92970ddf5282743hrc","ServiceType":"RpsCmd","SessionId":"04yebeay8cpxqwob4ze5u7y0iev07ffo","Step":"0"},"Header":{"MessageType":"MSG_CLI_NEED_CON_REQ","Version":"1.0"}}}
```

and answers `Content-Type: text/plain`:

```json
{"AgentProtocol":{"Body":{"AgentServerIp":"162.128.64.50","AgentServerLanIp":"10.210.0.2","AgentServerDNS":"162.128.64.50","AgentServerPort":"6611","ProtocolMode":"tcp"},"Header":{"CSeq":"3","Version":"1.0","MessageType":"MSG_CLI_NEED_CON_RSP","ErrorNum":"200","ErrorString":"Success OK"}}}
```

Two requests go out, one per channel: `ServiceType "RpsCmd"` with `Step "0"`, and
`ServiceType "RpsAV"` with `Step "8"`. Each carries its own `SessionId` and each is
answered with a different agent (`162.128.64.50` and `162.128.66.81` in this run);
the media tunnel is the one named by the `RpsAV` answer. `ClientToken` is a client
side value of the shape `c_` plus 25 lowercase alphanumerics, and `Authcode` is the
device's `rps` id from the account device list.

The tunnel handshake carries that same state, and both frames end with `XXEE`:

```
base64("<Authcode>:<ClientToken>:<SerialNumber>:<RpsAV SessionId>") + "XXEE"
```

Measured in one run: `RpsAV` answered `SessionId y8ekrmh5ud5wtaoak22y5o0vxz5l1qzg`,
and the 136-byte frame that reached `162.128.66.103:6611` decoded to
`aaaaaaaa-1a1b397d2:c_rpftjva4cpy4aphg09y083ya:7f92970ddf5282743hrc:y8ekrmh5ud5wtaoak22y5o0vxz5l1qzg`.
`crates/core/src/provider/p2p.rs` now builds both frames and parses the reply;
four tests pin them to those captured strings.

### 4C.7.1 What the access hop exposes, and why verification is off

The access server presents a self-signed certificate (`O=Internet Widgits Pty Ltd`,
RSA-1024, SHA-1, TLS 1.3) and OpenSSL refuses it with `Verify return code: 66
(EE certificate key too weak)`, before any policy of ours runs. `provider/rps.rs`
therefore posts with `SslVerifyMode::NONE`, scoped to that one private helper:
one host, one port, one request per session.

Turning verification off here adds no capability to an on-path attacker. The
host it dials comes from the status reply fetched over plain HTTP
(`crates/core/src/cloud/status.rs`), so an attacker who can rewrite that reply
already picks the destination. What actually crosses the hop is `Authcode`
(the device `rps` id), `ClientToken`, `SerialNumber` and `SessionId` — the same
four values the 6611 handshake then sends as plain base64.

The exposure worth naming is different: DVRIP inside the relay is plaintext, and
the credentials `examples/rps_probe` sends are the LAN ones (Sofia hash), which
are as good as the password itself, while the vendor SDK sends an AES login
instead (4C.9 `AESEncrypt:1`). **Boundary for the next change:** `stream()` must
not put a plaintext Sofia login on the relay; if the AES login cannot be
reproduced, that is a design decision to take with the board rather than a
default.

### 4C.8 DVRIP over the tunnel: login works, the media channel wants the encrypted exchange

Over the tunnel opened in 4C.7, plain DVRIP behaves as it does on the LAN up to
a point. Measured from this box with `examples/rps_probe`, whole chain in one
process and no vendor binary:

```
login    0.28s
devices  0.06s name=Pintu keluar masuk rps=aaaaaaaa-1a1b397d2
status   4.00s online=true
access   110.239.87.207:6510
agent    0.09s 162.128.64.68:6611
tunnel   0.02s error=200 dest=103.85.122.66 mtu=1400 accepted=true
dvrip    0.03s session=160 type=Some("IPC") channels=Some(1)
frame0   0.11s id=0x0586 len=67 {"Name":"OPMonitor","Ret":100,"SessionID":"0x000000…
```

The `OPMonitor` claim is accepted (`Ret 100`) when it is framed the way the SDK
frames it: message id `0x0585`, **channel 99**, end bit clear, body terminated by
a NUL instead of the newline the other JSON messages use, and carrying the
connection's own `SessionID` plus a fresh `DHParameter.RandomStrA`.

What does not happen yet is the media itself. In the SDK capture the same claim
is answered with a base64 body that decodes to 720 bytes of high-entropy binary
(printable ratio 0.37) and is followed by a client frame with message id
`1000`, channel 99, and 384 bytes of the same shape: that pair is the DHAV key
exchange, and our claim (which the device answers in plaintext) never reaches it.
So the remaining work in this milestone is that exchange, not the transport.

### 4C.9 The media channel: client frames are AES, device frames are plaintext

Captured again on 2026-10-27 with the camera unreachable from this box (TCP 554
and 34567 closed, ICMP 100% loss), so the whole exchange is cloud-only. Both
directions of the tunnel, parsed past the handshake:

```
client -> relay  0x0585 ch=99  {"Name":"OPMonitor","Action":"Claim",... "RandomStrA":"o1yR","SessionID":"0x000001869f"}
relay  -> client 0x0586 ch=0   961 bytes, base64 of 720 bytes of binary
client -> relay  0x03e8 ch=99  513 bytes, base64 of 384 bytes (encrypted login)
relay  -> client 0x03e9 ch=0   128 bytes, plaintext {"AliveInterval":30,...,"Ret":100,"SessionID":"0x00000098"}
client -> relay  0x03fc/0x0550/0x0412/0x0582  89-321 byte base64 control messages
```

So the asymmetry is the useful part: everything the client sends after the claim
is ciphertext, and the device's `1001` answer is plaintext. The SDK logs the
parameters it uses (`ProtocolNetIP.cpp:1293`):

```
NewLoginPTL[Communicate:0,AESEncrypt:1,DevNetType:1,User:<device user>,PwdLen:<n>,
             Token:<16 chars, the same in every run>,Pid:,RandomStrLen:4/0,
             PwdAppendRandomStr:0,TokenAppendRandomStr:0]
```

That token is **identical in all thirteen harness runs** recorded in `sdk.log`,
hours apart and on different relays, so it is stable material rather than
something the claim reply hands out. Direct tries with it as an AES-128 key, with
`md5`/SHA derivations of it, and with the account `AdminToken` (and XOR
combinations) do not open the 720-byte claim reply, so the key derivation is
still unproven.

Next measurement, already tooled: the harness already interposes at an exported
symbol by patching its prologue, so the same trick on the statically linked
`AES_set_encrypt_key`/`EVP_EncryptUpdate` yields the exact key and plaintext the
SDK feeds the cipher, which settles the derivation instead of guessing it.

### 4C.10 Where the media key hides: not OpenSSL's AES, and not a simple derivation

Two negative results that narrow it, both from a harness run with the inline
hook extended over the AES entry points (`AES_set_encrypt_key`, `AES_cbc_encrypt`,
`EVP_EncryptUpdate`) besides `SSL_write`:

1. Every `AES_set_encrypt_key` the run hits is 32 bytes of session material and
   belongs to TLS record encryption; `AES_cbc_encrypt` is never called at all.
   So the media login does not go through the OpenSSL primitives the SDK exports.
2. A crib attack on the client's 384-byte media login (73 candidate keys:
   `md5`/`SHA-256` over every pair of token, `RandomStrA`, password, password
   hash and serial, plus the raw token, each under ECB and CBC with three IV
   choices) produces no `OPLogin` plaintext.

The binary explains why: `libFunSDK` carries its own class for this,
`MNetSDK::CDataEncDecAES` (`EncData`/`DecData`/`AESEncData`), with RTTI for it in
the library but no exported symbol, so neither `LD_PRELOAD` nor a lookup by name
reaches it.

Next measurement: recover the vtable through the RTTI structures the library
does publish (`_ZTVN7MNetSDK14CDataEncDecAESE` is present as a name string), then
patch `EncData` the same way `SSL_write` was patched and log the key and the
plaintext it is handed. That settles the derivation instead of guessing it.

### 4C.11 A plaintext login on the media channel is accepted and still does not stream

Three orderings were tried from this box with the camera unreachable on the LAN,
all over the tunnel of 4C.7, and none produced a media frame:

1. `OPMonitor` claim, then wait. Answer: `{"Name":"OPMonitor","Ret":100}`, and
   nothing else inside six seconds.
2. Claim, then a **plaintext** login on the same channel 99 (message id 1000, end
   bit clear, body NUL-terminated, the same JSON the LAN path sends). Answer:
   `{"Name":"OPMonitor","Ret":100,"SessionID":"0x00000100"}` — accepted, a new
   session id handed back — and still no frame.
3. Claim, login, then a second claim. Answer: `Ret 117` (device error), and the
   following control read is refused with `Ret 103`.

So the device treats the plaintext login as a session but only streams to a client
that has done the encrypted login of 4C.9: the SDK's own capture shows ciphertext
on every client frame after the claim and media arriving right behind its `1000`
login. Claiming twice also poisons the channel, which is why the sequence has to be
built once and in order.

That makes the encrypted login the single gate for live video, and 4C.10 records
what has already been ruled out about its key.

### 4C.12 The media cipher is not the CDataEncDecAES object we can reach

The RTTI for `MNetSDK::CDataEncDecAES` does lead to its vtable without symbols:
the type name sits at `.rodata` `0x5b3177`, the only relocation whose addend is
that address is at `0x1f40898` (so its typeinfo starts at `0x1f40890`), and the
only relocation whose addend is `0x1f40890` is at `0x1f40838` — which puts the
vtable at **`0x1f40830`**, with `D1`/`D0` at `+0x10`/`+0x18` and five slots
after them (`0xf2e1cc`-range destructors, then `0xf2e1d8`, `0xf228a4`,
`0xf228bc`, `0xf2e464`, `0xf2e704`).

Patching those five slots at runtime inside the harness (data patch, base from
`dl_iterate_phdr`) and running a full live session produced **zero calls**: no
slot fired even once. So the media login is not encrypted through this object in
this code path, and hooking it cannot yield the key.

What that leaves, in order of cost: the constructor that stores the key
(`CDataEncDecAES(char const*, int, void const*, int)`, address still unknown
because the code that stores `0x1f40830` was not located), or a different
encryption object entirely, or the `CProtocolNetIP` layer that builds the login.

### 4C.13 The path is also a diagnostic

`GET /api/cloud-path?device=<serial>` walks the same steps as a live attempt and
answers the result, per stage (`access`, `agent`, `relayAccepted`, `relayError`,
`destIp`, `mtu`, `detail`). It never logs in to the device, so nothing crosses the
relay beyond the handshake of 4C.7.

Measured over HTTP from this box against the real account, one session:

```
POST /api/login                                       200 in 0.3 s
GET  /api/stream?device=<sn>&channel=1&quality=hd      501 in 0.0014 s
GET  /api/cloud-path?device=<sn>                       200 in 0.40 s, 0.38 s, 1.46 s
   {"online":true,"access":"110.239.87.207:6510","agent":"162.128.64.191:6611",
    "relayAccepted":true,"destIp":"103.85.122.66","mtu":"1400",...}
```

The three rounds were answered by three different agents (`162.128.64.191`,
`162.128.64.50`, `162.128.66.64`), which is the per-session assignment of 4C.7
showing up in the diagnostic.

### 4C.14 The media login is a token login, and the SDK log spells it out

The SDK's own log carries the login it builds for the media channel
(`ProtocolNetIP.cpp:1293` and the frame that follows it):

```
NewLoginPTL[Communicate:0,AESEncrypt:1,DevNetType:4,User:zhafronadani@gm,PwdLen:13,
             Token:o5rBSnodWCpN8VAa,Pid:,RandomStrLen:4/0, ...]
{"EncryptType":"TOKEN","LoginType":"DVRIP-FutureHome","UserName":"o5rBSnodWCpN8VAa",
 "PassWord":"","CommunicateKey":"8291FD6A0A7EF5F78495CB0255F4E862A4382321D951D2257176E83E2573DE5F...",
 ...}
```

Two things follow. The media channel does **not** use the account password: it
logs in with `EncryptType "TOKEN"`, puts the 16-character token in `UserName`,
sends an empty password, and passes an RSA `CommunicateKey`. And the device
advertises exactly which algorithms it expects:

```
{"Bits":1024,
 "DataEncryptionType":{"AES":true,"AESV2":true,"VEKEY1":true},
 "EncryptAlgo":"RSA_V1.5",
 "LoginEncryptionType":{"MD5":true,"NONE":true,"RSA":true,"T":...}}
```

So the earlier hunt for an AES key (4C.9-4C.12) was aimed one layer too low: the
gate is the **token** (from the account's device token, the same kind of value
`/deviceToken/get` already returns) plus an RSA-1.5 session key, not a password
hash. That reframes the remaining work in 4C.11: build the token login and the
`CommunicateKey`, then see whether media flows.

The same log also shows the audio channel being negotiated as `service_type
"RpsVoIP"` with its own `local_port`, which is the shape two-way audio will need.

### 4C.15 The token login is accepted, and the session key is what is still missing

Tried on the tunnel with the account's device token (the `deviceToken.AdminToken`
value, 44 base64 characters) as `UserName`:

```
claim        -> {"Name":"OPMonitor","Ret":100}
login TOKEN  -> {"Name":"OPMonitor","Ret":100,"SessionID":"0x00000250"}   accepted
frames       -> none inside six seconds
```

So `EncryptType "TOKEN"` with an **empty** `CommunicateKey` is accepted by the
device but buys no media. The SDK sends a non-empty `CommunicateKey` (hex, RSA
`RSA_V1.5`, device advertises `Bits 1024`), which is the session key the media
channel is encrypted with — and the one thing worth generating ourselves next,
since a self-chosen AES key wrapped to the device key is a key we would know.

The device public key is not in the caps payload `CapsClient` parses today
(`grep PublicKey|Modulus|Bits` over the caps types: no hits), so the next
measurement is to capture the key exchange or read it from the device's own
system-info reply.

`Connection::login_token` in `crates/core/src/dvrip/client.rs` reproduces the
attempt, and `examples/rps_probe` drives it when `PROVIEW_DEVICE_TOKEN` is set.

### 4C.16 The RSA call the SDK makes carries a 16-character session key

Hooking `RSA_public_encrypt` in the harness (same inline-patch mechanism as
`SSL_write`) shows three calls per live session:

```
flen=12  padding=1  <a password-shaped value, redacted>
flen=8   padding=1  <8 random characters>
flen=16  padding=1  iq5H3765j5t04mO6      (a second run: ke930p9mR1kxcL7Q)
```

The 16-byte one changes on every run and has exactly the shape of the
`CommunicateKey` token `NewLoginPTL` reports (`o5rBSnodWCpN8VAa`): the client
picks the session key itself and RSA-wraps it for the device. That is the piece
this design was missing — the media session key is **ours to choose**, so once it
crosses the wire we know it and can decrypt what the device sends.

What is still not captured is the device public key the wrapper uses. The next
step is a corrected dump in the same shim (`RSA_get0_key` + `BN_bn2hex` on the
`RSA*` argument; the first attempt broke on an escaped newline in the C source
and the run fell back to the previous binary), which turns this into a complete
`CommunicateKey` construction in Rust.

### 4C.4 What that means for the implementation

1. The status cluster that holds the device reports `ServerIP:ServerPort`; that
   is the RPS access server (`110.239.87.207:6510` on the test camera). The UDP
   8765 discovery list is **not** part of this path — the agent is assigned per
   session, not looked up (4C.6).
2. `POST /` on that host with the `AgentProtocol` request of 4C.7, `ServiceType
   RpsAV`, and read `AgentServerIp`/`AgentServerPort` from the answer.
3. `tcp <agent>:6611` → base64 `<rps>:<client token>:<serial>:<RpsAV SessionId>`
   plus `XXEE` → base64 JSON with `ErrorNum 200` and `MTU`.
4. DVRIP over the tunnel: login with the device credentials and the `OPMonitor`
   claim (4C.8). The `rps` id comes from `msdlistCode`, so no new account call is
   needed; the media frames themselves still need the encrypted login of 4C.9.

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

Two-way audio uses the DVRIP backchannel, not RTSP: the SDP carries
`m=audio 0 RTP/AVP 8` (`PCMA/8000`, `trackID=4`) with no backchannel and the ONVIF
device service exposes no audio output, while the device reports
`SupportTwoWayVoiceTalk=true`. The message ids come from the shipped SDK —
`CProtocolNetIP::NewMediaControlPTLE` (0xf2816c) for the `OPTalk` body and the
`1434` claim against `1430` start/stop, `CProtocolNetIP::NewMediaDataPTLE`
(0xf29ee8) for the `1432` audio frames whose 8-byte header is
`00 00 01 FA | 0E | 02 | length` followed by G.711 A-law, 320 samples (40 ms) per
frame at 8 kHz. The browser captures through an AudioWorklet at 8 kHz and the
server encodes A-law, so no capture plugin is needed. Measured 2026-10-27: claim
`Ret 100` in 477 ms, 50 frames in 2.00 s and 0.079 ms mean write time to the
device over the vendor relay. In the other direction the device streams its own
audio back while the claim is open: a 90 s session left 743,328 bytes unread in
the kernel receive queue, about 8.8 kB/s or 25 frames of 348 bytes a second. The
build now drains and counts that stream (`inbound_bytes`, `inbound_frames` in the
report) instead of letting it fill the queue, but it does not play it. What is
still unproven is acoustic: whether the camera speaker plays what it receives
needs a person standing next to it.

Multi-channel: the device reports one media profile and `ChannelNum 1`; the dual
lens arrives as one frame, so a two-viewport grid needs cropping, not a second
channel.

## 9A. PTZ over DVRIP: the tour, the motorised lens, and what the acks are worth (2026-10-27)

There is no route from this box to the camera LAN (192.168.0.0/24 answers
nothing), so every number here was taken through the cloud relay chain: the RPS
access server answered `110.239.87.207:6510`, handed out the agent
`162.128.64.146:6611`, and the relay reply was dest `103.85.122.66`, MTU 1400.
DVRIP login inside that tunnel succeeded on sessions 282-298 as `IPC` with
`ChannelNum 1`. The runs and their artifacts are in
`~/re/proview-cctv/ZHA-4012/`.

Frames: `OPSNAP` (message 1560) on the control channel returns a 640x720 JPEG,
which is the two lenses stacked (top = gun, bottom = ball). Every figure below
is the mean absolute luma difference over one half of that frame, against a
noise floor taken in the same session.

| Pair, same session, camera untouched | top | bottom |
|---|---|---|
| `00-before` to `01-before-second`, 1 s apart | 0.13 | 0.59 |
| five frames 1 s apart, second run | 0.13-0.14 | 0.69-0.72 |

`DirectionRight`, `Step 3`, 400 ms, then the same command with `Preset -1` to
stop, once per PTZ channel:

| Move | top | bottom |
|---|---|---|
| channel 0 | 0.19 | **38.17** |
| settle after channel 0 | 0.15 | 1.82 |
| channel 1 | 0.19 | 1.09 |
| total for the run | 0.34 | 38.21 |

So the motor answers on PTZ channel 0 and it moves the lower half only: the
upper half never leaves its noise floor and channel 1 is inert.

Two examples reproduce this without a LAN route:
`cargo run --example relay_bridge -p proview-core` opens the RPS chain and serves
it on `127.0.0.1:34567`, which is what `PROVIEW_DEVICE_HOST` points at, and
`cargo run --example ptz_probe -p proview-core` with
`PROVIEW_PHASE=lens|tour|config` writes the snapshots and the replies into
`PROVIEW_OUT`.

### 9A.1 The tour runs and stops, and the device says so in its own config

`Uart.PTZTour` was `null` on this camera, so a `StartTour` with no tour line is
acknowledged and does nothing: ten frames over seven seconds stayed at
0.15-1.03 on both halves. Building the line first is what makes a tour real.

| Step | Reply | `Uart.PTZTour` after |
|---|---|---|
| `AddTour` tour 1, preset 1, index 0 | `Ret 100` | unchanged |
| `AddTour` tour 1, preset 2, index 1 | `Ret 100` | `[[{"Id":1,"Name":"","Tour":[{"Id":1,"Name":"","Time":3},{"Id":2,"Name":"","Time":3}]}]]` |
| `StartTour` tour 1 | `Ret 100` | unchanged |
| `StopTour` tour 1 | `Ret 100` | unchanged |
| `ClearTour` tour 1 | `Ret 100` | `[[]]` |

With the two presets in the line (the camera already carried `biasa` and
`biasabaru` under `Uart.PTZPreset`), `StartTour` moved the lower lens between
them and `StopTour` stopped it. Bottom half, mean absolute luma change against
the previous frame, 700 ms apart:

| Frame | bottom | top |
|---|---|---|
| 0, first after start | 73.10 | 0.64 |
| 1 | 29.62 | 0.67 |
| 2 | 6.84 | 0.45 |
| 3, dwell at a preset | 1.30 | 0.40 |
| 4 | 19.15 | 0.38 |
| 5 | 70.77 | 0.28 |
| 6 | 16.58 | 0.33 |
| 7 | 1.90 | 0.22 |
| 8 | 40.07 | 0.22 |
| 9 | 3.47 | 0.15 |
| stop + 0.7 s | 1.57 | 0.18 |
| stop + 1.4 s | 1.13 | 0.15 |
| stop + 2.1 s | 1.02 | 0.15 |
| stop + 2.8 s | 0.97 | 0.15 |

The top half never leaves 0.15-1.07 through the whole tour: the gun lens is
fixed while the ball cycles.

### 9A.2 The ack is not evidence

Every `OPPTZControl` on this firmware answers `Ret 100` before anything moves.
Tried in one session: `StartTour`, `StopTour`, `StartAutoCruise`,
`StopAutoCruise`, `StartScan`, `EndScan`, `StartPattern`, `StopPattern`,
`GotoPreset`, `SetPreset`, `ClearPreset`; all `Ret 100`, including the ones that
do nothing (9A.1 shows the difference). The API therefore reports the device own
configuration (`Uart.PTZTour`, `Uart.PTZPreset`) as state, and the frames stay
the only proof of motion.

### 9A.3 The command surface is the app own

The APK carries it in `com.lib.sdk.bean.tour.OPPTZControlBean`: `AddTour`,
`ClearTour`, `DeleteTour`, `StartTour`, `StopTour`, a `Parameter` of `Preset`,
`PresetIndex`, `Step`, `Tour`, `TourTimes`, the tour list at JSON path
`Uart.PTZTour`, the preset list at `Uart.PTZPreset`, and a tour-end reply with
message id 2141. `EPTZCMD` in the same APK is the numeric alias of the same
surface: `EXTPTZ_STARTPANCRUISE` 20, `EXTPTZ_STOPPANCRUISE` 21,
`EXTPTZ_ADDTOLOOP` 26, `EXTPTZ_POINT_LOOP_CONTROL` 28,
`EXTPTZ_POINT_STOP_LOOP_CONTROL` 29.

### 9A.4 The motorised lens is named by the device

`Ptz.GunBallLocate[0]` is the device own description of the gun and ball pair:

```json
{"AdjustHeightSteps": 305, "AdjustWidthSteps": 1010, "Enable": true,
 "HeightSteps": 285, "PtzLocate_CenterX": 1735, "PtzLocate_CenterY": 1028,
 "WidthSteps": 515, "motorSpeedX": 13, "motorSpeedY": 9}
```

`Enable: true` with a locate rectangle and motor speeds is the device declaring
exactly one motorised sensor in the pair, and the measurement above says that
sensor is the lower half. `Uart.PTZControlCmd[0]` additionally carries
`MirrorOperation: true`, `FlipOperation: false`. `/api/lens` reports
`ptzSource: "device"` with this block as `ptzEvidence` instead of the measured
default it used to guess.

### 9A.5 How the route reads the binding without stalling the picture

`/api/lens` sits on the path that starts the live stream (`web/app.js` awaits it
before it asks for the stream), so it never waits on the camera: it answers from
the cache, falls back to the measured default when the cache holds nothing, and
asks for a refresh in the background. One refresh is one DVRIP session bounded
by `tokio::time::timeout` at 10 s. The budget covers the measured path with
headroom: 1.0 s for the whole probe through a local relay bridge, and 3.2 s for
the direct chain (0.28 s envelope login, 1.02 s tunnel, 1.87 s DVRIP login).
While the probe runs the route keeps answering in 1.3-2.3 ms with the measured
default, then replies `device` once the answer lands.

Every outcome is stored, not just success: a device answer lives 10 minutes
(`GunBallLocate` is static configuration), a failure or a timeout lives 1
minute so a camera that comes back is picked up without a restart. One probe is
in flight at a time, so a burst of requests cannot multiply sessions; before
this, a silent peer held the request for as long as the socket lived and a dead
device was dialled again on every call.

## 9B. Playback download over the tunnel: DownloadStart, and the audio that is not there (2026-10-28)

`OPPlayBack` carries two more actions beside `Claim`/`Start`/`Stop`: `DownloadStart` and `DownloadStop`, sent on the same control id `1420` with the same file body (`crates/core/src/dvrip/playback.rs`). They are not a different protocol; the file arrives as the same DHAV record framing, on a different message id.

| Fact | Measured |
|---|---|
| Data message id | `0x0592` (`DOWNLOAD_DATA`). Playback video uses `0x058e` (`PLAYBACK_DATA`). Both carry `00 00 01 F9/FA/FC/FD` records |
| Claim | `Ret 100` in 0.20 s, then `DownloadStart` acknowledged in 0.00 s |
| Whole file | `/idea0/2026-10-24/001/10.34.00-10.40.00[R][@e19][0].h264`, 1,313 chunks, **42,967,040 bytes** — `FileLength` 41,960 KiB × 1024 exactly, byte for byte on the wire |
| Wall clock | **60.84 s** for 42.97 MB = **706 KB/s**, six times the 119 KB/s the file's own duration and size suggested |
| Records | Key 125, Delta 4,366, Pad 8,982, Info 126 = 13,599 records, 10 bytes skipped by the record scanner |
| Video payload | Key and Delta bodies concatenated = **39,981,987 bytes**. `ffprobe`: HEVC 2304×2592, `nb_read_frames` 4,491, `r_frame_rate` 12/1 — 374 s of video |
| Audio | none. 13,599 records is every record in the file, and all of them classify as Info/Pad/Key/Delta. No audio magic value appears, on download or on playback |

The gap between 42,967,040 wire bytes and 39,981,987 payload bytes is the device's own storage framing: 8,982 Pad records of 328 bytes plus the 8/16-byte record headers on every record. Emitting the wire bytes as a file produces something no decoder opens; emitting the Key and Delta bodies produces the elementary stream the camera recorded, which is what `GET /api/playback/download` streams (`Content-Disposition` carries the device file name, `X-File-Bytes` carries `FileLength` × 1024). The response is chunked: the payload length is only known once the device stops.

Audio is therefore a boundary of this channel, not a missing implementation: the camera records video on this stream and there is no record kind to carry a G.711 frame here, unlike the two-way talk path (`1432` audio, section 9). A caller that wants playback audio has to come back for a different device surface, not for a flag on `OPPlayBack`.

```
PROVIEW_EMAIL=… PROVIEW_PASSWORD=… PROVIEW_DEVICE_SERIAL=… \
PROVIEW_BEGIN="2026-10-24 10:34:00" PROVIEW_END="2026-10-24 10:34:02" \
PROVIEW_DOWNLOAD=1 PROVIEW_WHOLE=1 PROVIEW_DUMP=/tmp/clip.h264 \
timeout 600 cargo run -p proview-core --example playback_probe
```

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

## 10D. Device bind on the account envelope (`mdadd`, `mdrm`), and the QR payload codec

Measured 2026-10-27, issue ZHA-4008, against the live account and cloud; the
strings come from `libFunSDK.so` (arm64-v8a, the APK's own copy).

### 10D.1 The bind call is `mdadd`, not `/createbind`

The `XMAccount` module carries the whole device list surface next to
`msdlistCode`: `mdadd`, `mdrm`, `delDevices`, `mdedit`, `mdeditPass`,
`useredun`, `devicesTransOut/In/Info/Cancel` and `hasDevicesTrans`. `mdadd`
builds `uuid=%s&username=%s&password=%s&userId=%s&nickname=%s&type=%s&ip=%s&port=%s`
(0xf3e798) and posts it as `mdadd` on the same signed envelope as
`loginExist`/`msdlistCode`; `mdrm` is `uuid=%s` (0xf3ebdc). The `/createbind`
and `/listbind` strings sit in the RPS relay agent instead
(`[rps agent]`, `tcp_proxy_agent_client.cpp`, `CDeviceAgent::ListBind`), which
is the media tunnel, not the account.

Wire shape, exercised from this box:

    POST https://rs.xmeye.net/mdadd/v3/<7-digit counter><millis>/<sig>.rs
    uuid: <app uuid>   appKey: <app key>   Authorization: <account access token>
    body: HEX( AES-128-ECB-PKCS7( "uuid=…&username=…&password=…&userId=…&nickname=…&type=0&ip=0.0.0.0&port=34567", body_key(tsid, app secret) ) )
    reply: {"code":2000,"msg":"SUCCESS","data":""}

Replies seen, in order of the measurements:

| Body | Vendor reply |
|---|---|
| `uuid=<serial>` only | `4105 DEV_USERNAME_FAIL` |
| `uuid=<serial>&userId=<account id>` | `4105 DEV_USERNAME_FAIL` |
| full body, real serial, real camera password | `4101 DEV_ALREADY` |
| full body, real serial, wrong camera password | `4101 DEV_ALREADY` (the already-bound short circuit wins) |
| full body, `uuid=no` | `4104 DEV_UUID_FAIL` |
| full body, fictional `AASN000000000999` | `2000`, and `msdlistCode` then returns **two** devices |
| `mdrm` `uuid=AASN000000000999` | `2000`, `msdlistCode` back to one device |

Two consequences worth keeping: the cloud does **not** validate that a serial
belongs to a real device on bind (`2000` for a made-up one), so a bind result is
not a device check — `msdlistCode` is; and `4101` means "this account already
holds that serial", which is why a wrong camera password on an already-bound
camera still answers `4101`. The account was left exactly as it was found: the
fictional device was removed again with `mdrm` and the list returned to one
camera.

### 10D.2 The QR payload codec

`DevQrCodeDlg` is the generator: the owner path builds
`{devType,userId,loginName,pwd,devId,shareTimes}` and calls
`FunSDK.EncGeneralDevInfo(json)`, the other path calls
`EncDevInfo(sn,user,pwd,devType)`; both end in `XEnCodeStr`. Scanning runs the
inverse, `XDeCodeStr`, and splits the plain text on commas; the SDK's own
`FUN_DecGeneralDevInfo` also accepts a plain `sn,user,pwd,type,time` string
(it checks the character classes before decoding).

`crates/core/src/cloud/bind.rs` implements both directions. The pipeline,
read off `XEnCodeStr(char*, char const*, int)` at 0xc374a8 and its helpers:

1. `offset = time() % 744` (the two-argument wrapper, 0xc3728c).
2. Each byte becomes `%03d` of `byte + offset`, so the digits stay printable.
3. Every digit is shifted by `digitsum(offset) % 10`.
4. `%03d` of the offset is appended, unshifted.
5. Digits are packed in pairs, `value = d0*10 + d1`, by `To62` (0xc36fec):
   `0..25 → a..z`, `26..51 → A..Z`, `52..60 → 1..9`, and `≥61 → '0'` followed by
   `To62(value-61)`.
6. `DevInfoEncode` (0xc36bdc) shifts every character inside its own class —
   digits `% 10`, lower and upper `% 26` — by `key[index % len(key)]` with the
   key `DecInfoEncode`.
7. A literal `z` closes the string.

Vectors produced by the shipped library under the bionic harness (qemu-user,
`re/qemu/harness_qr*.c`) and reproduced byte for byte:

| Plain | Offset | Blob |
|---|---|---|
| `a` | 0 | `z1evz` |
| `abc` | 0 | `z1e3Rh1Ugz` |
| `abc` | 1 / 2 / 3 | `a1Zvbiiz` / `lkrBKaz` / `HFDMEbz` |
| `abc` | 334 | `HlBMNGz` |
| `7f92970ddf5282743hrc` | 0 | `vWxa0hFw0e5a1kxfwidLPl8FbV9cgzrM1Dgvz` |
| `AASN000000000001,,,2,1793109504` | 312 | `8DIC0Ga40A1eX8C9dG2M9oV1B1yX9A0gH9L9pX1UAh3KBJ9KYI0CDw9A0KI9L9pv1VqV3Lbtz` |

`EncDevInfo` replaces an `admin` login name with an empty field and appends
`time(0)` as the fifth column, which is why the vendor blob for the real serial
with a harness password decodes to `7f92970ddf5282743hrc,,TestPass1,0,1793109504`
(`TestPass1` is the input in `re/qemu/harness_qr.c`, not the camera password).

### 10D.3 Sign-out

`IXMAccount::XM_Video_Logout_XMAccount` (0xf40808) is a cookie call:
`POST /webservice/app/user/logOut` with `Host` from `MI_SERVER`
(`rs.xmeye.net:443`), `Cookie: <the session that /webservice/app/user/logInByAccount
set>`, `Connection: close`. That cookie belongs to the legacy portal login whose
envelope is still unsolved here, so the call cannot be authenticated from a v3
session. Measured from this box: `/webservice/app/user/logOut` answers
`5005 PARAM_FAIL` on `rs.xmeye.net` — and so does the control path `/zzz`, so the
gateway tells us nothing about the route. The v3 account service has no
sign-out route at all: `logOut`, `logout`, `userLogout`, `signOut`, `quit`,
`userexit` and the control `zzzNoSuchMethod` each answer the same Spring 404
with the path echoed, while `mdadd` on the same shape answers a real code.
`AccountUnBindingWX` is a WeChat unlink (`userunbindWx`), not a sign-out. What
the client does instead: `POST /api/account/signout` clears the server session
and the remembered token and records the vendor's raw reply in its answer.

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
