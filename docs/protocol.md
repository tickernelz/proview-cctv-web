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

## 5. Open items

1. The CSS request envelope for account login (message type, body shape, digest).
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
`portal`, `captcha`, `session`, `logout`, `profile`, `devices`.

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
