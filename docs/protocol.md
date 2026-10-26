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

The device-facing transport in `src/dvrip/` follows the framing that the SDK's
`CNetTCP`/`CDevProtocol` classes speak: a fixed 20-byte header
(`0xFF`, then session, sequence, channel, end-flag, message id, body length, all
little-endian) followed by a JSON body.

- Message ids used: `1000` login, `1001` logout, `1006` monitor start,
  `1007` monitor stop. **These ids come from public DVRIP documentation, not
  from this APK** — the APK drives the same device through `libFunSDK.so`, whose
  ids are inside native code and were not extracted. Treat the ids as
  unverified against this device until the probe reports a real session.
- Login sends `{EncryptType, LoginType: "DVRIP-Web", PassWord, UserName}`. A
  `Ret: 200` reply with a `Realm` triggers the second round with the realm
  digest; `Ret: 100` means accepted. Both digest variants are attempted in a
  fixed order, and the reply's `SessionID` (hex or decimal string) becomes the
  session used by later frames.

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
