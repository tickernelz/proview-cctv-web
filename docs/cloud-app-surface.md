# App cloud surface: what the iOS client talks to

Provenance: Proxyman capture of the App Store build (`com.szyarsor.proviewcctv`,
version 1.0.2) on an iPhone (iOS 18.7), exported as `filtered_flows_10-27-2026-07-17-44.har`
on 2026-10-27. 290 entries, 174 of them decrypted HTTP, window
`2026-10-26T23:55:44Z` .. `00:17:31Z`. Everything below is read from those entries
unless it is marked as a probe or as inference.

This surface is *not* the v2 portal of section 10. The app never calls
`api.xmeye.net`; it talks to the jftechws / xmcsrv / secu100 families.

## 1. Identifiers the app carries

| Name | Value | Where it appears |
|---|---|---|
| app access key | `559648008040759296` | `api.jftechws.com/privacy/*` body `accessKey` |
| package | `com.szyarsor.proviewcctv` | `pkg` in privacy and caps bodies, `appCaps?pkg=` |
| mall app key | `3a59da9c8ffd41a64d492aa064b1dcf6` | `userToken/login`, `getH5ShowEntrance`, `campaign/get/info` |
| mall scheme | `icsee.boss.jftech.com` | `userToken/login` body `appScheme` |
| account id | `c6f8c33b29c95774c14bae9436e721c8` | privacy `userId`, `reqCaps.sn`, JWT claim `account` |
| device serial (uuid) | `7f92970ddf5282743hrc` | `asMsgQ.sn`, `latestCacheMulti`, share row `uuid` |
| share owner id | `as5670b4b61e7940809b82a60b11246747` | share row `shareAccountId` |
| device pid / hw | `A90A007F3000000C` | `asMsgQ` reply, share row `pid` |

The account id doubles as the `sn` for account-scoped caps calls, and the device
serial doubles as the `uuid` and as the key of the firmware-cache call. Neither is
interchangeable with the other.

## 2. Login

```
POST https://boss22-api.xmcsrv.net/account/userToken/login
content-type: application/json
{"request":{"authorization":"<JWT>","appKey":"3a59da9c8ffd41a64d492aa064b1dcf6",
            "appScheme":"icsee.boss.jftech.com","appVer":"1.0.2"}}
-> {"code":2000,"msg":"success","data":{"accessToken":"<same JWT>",
    "publicKey":"<RSA-1024 SPKI, base64>","area":"as"},"timestamp":...}
```

The JWT is HS256 with claims
`{"isoCode":"","iss":"rss","appKey":"3a59da9c…","exp":<iat+30d>,"type":"com","iat":…,"account":"c6f8c33b…"}`.
The reply echoes the same token, so this endpoint *exchanges* a token the client
already holds for a mall session; it does not mint one from a password.

Open: where that JWT is minted. `pub-dscn.secu100.net` is the only vendor host in
the capture that stayed a bare `CONNECT` (20 entries, e.g. `00:08:00`, `00:08:16`)
while every other vendor host decrypted, and the token's `iat` lands 19 s before
this call, i.e. inside that host's window. Inference (not proven): the minting
request lives on the pinned `pub-dscn` connection, so the login envelope is behind
certificate pinning.

That gap was closed a different way on 2026-10-27: the vendor's own
`libFunSDK.so` was run out of tree and the login envelope fell out of it
(email + password → `accessToken`, the platform exchange, and the signed
`/v3/<tsid>/<sig>.rs` path with a 96-hex AES body). `protocol.md` sections 3A
and 3B record the derivation, and `crates/core/src/cloud/envelope.rs` is the
client this project ships: `POST /api/login` needs no vendor binary, chroot or
emulator, and the account's device list follows on `GET /api/devices`.

## 3. Device list

```
POST https://as-ams.jftechws.com/mdsharemylist/v1/<ts-id>/<sig>.rs    body: shareUuid=
-> {"msg":"SUCCESS","code":2000,"data":[{ "id","uuid","shareTime","acceptTime","ret",
     "powers":"{\"devInfo\":\"<base64, encrypted>\"}","account":"ar***@gmail.com",
     "shareAccountId","pid","permissions":[{"label":"DP_AlarmPush","enabled":true,…}] }]}
POST https://as-ams.jftechws.com/mdsharelist/v1/<ts-id>/<sig>.rs     body: (empty)
-> {"code":2000,"data":[],"msg":"SUCCESS"}    (23 calls, all with empty data)
```

The `<ts-id>` is a 20-digit number ending in the epoch milliseconds; the hash is a
per-request signature, so neither can be replayed from the capture. The share row is
the only place the capture shows a device the account can open, and its `devInfo` is
encrypted.

## 4. Device capabilities, and this one needs no authentication

```
POST https://caps.xmcsrv.net/api/asMsgQ
{"method":"reqCaps","body":{"caps":["xmc.css","xmc.voes","pms.voes","pms.sms",
  "net.cellular","xmc.ais.offduty","xmc.ais.recface","xmc.ais.birds","xmc.ais.timealbum"],
  "sn":"<device serial>"}}
-> {"ret":200,"msg":"Success","method":"reqCaps","sn":…,"mfrsOemId":"G51","forceOemId":"G51",
    "bossOemId":"G51","factoryOemId":"G51","lastWanIp":"103.85.122.66",
    "model":"XM535_X6E-WEQ_8M","hw":"A90A007F3000000C","pid":"A90A007F3000000C",
    "sw":"V5.01.R02.G51A07F3.10010.346732.0000000","swVer":"2024-11-01 19:21:23",
    "pwd":"47.86.110.52","caps":{…}}
```

Probe 2026-10-27, this host, no cookies, no token, same body as the capture: the
reply came back whole (`ret:200`, model, firmware, `lastWanIp`, `pwd`). So device
model, firmware, WAN address and the capability block are reachable from a serial
alone, without an account session. `pwd` here is the cloud relay password the
device registers with, not the camera login.

Account-scoped sibling, with the account id as `sn`:
`POST https://caps-as.jftechws.com/api/reqCaps` `{sn, ver:"1.0.2", caps:["xmc.cloud.disk"]}`
-> `{"ret":200,…,"pwd":"47.86.110.52","caps":{"xmc.cloud.disk.support":true,
"xmc.cloud.disk.enable":false,"xmc.cloud.disk.storagespace":0,…}}`.

## 5. Feature flags

`GET https://caps-as.jftechws.com/api/appCaps?pkg=com.szyarsor.proviewcctv&os=ios` and
`GET https://caps.xmcsrv.net/api/appCaps?pkg=com.szyarsor.proviewcctv` both answer

```
{"ret":200,"stat":{"cee":true,"bmn":false,"ade":false,"trc":true,"bglive":false,
 "ais":[],"pkg":"com.szyarsor.proviewcctv","xmc":true,"spk":[],"vvt4g":300,"thd":[],
 "disk":false,"vvt":600,"jfclaw":false,"vs":false}}
```

Both are readable without a session as well.

## 6. The rest of the surface

| Call | Body | Reply |
|---|---|---|
| `POST api.jftechws.com/privacy/query` | `{accessKey,pkg,lan,userId}` | `5005 No APP Privacy` |
| `POST api.jftechws.com/privacy/agree` | `{accessKey,pkg,lan,operateTime}` | `12501 the system error.` |
| `POST app-support.xmcsrv.net/api/open/problem/customer/unReadMessage.do` | `{userId,packageName}` | `{"code":2000,"data":{"replyUnreadCount":0,…}}` |
| `POST app-support.xmcsrv.net/api/open/h5/getH5ShowEntrance` | `{index,appKey}` | `{"code":200,"data":{"showEntrance":true}}` |
| `POST api-integral-mall.xmcsrv.net/integral/coin/queryUserCoins` | (token bearer) | `{"code":2000,"data":0}` |
| `POST promotion.xmeye.net/appstore/api/advert/campaign/get/info` | `{username:<email in clear>,"versionName","appkey","campaignType":1}` | HTTP 404 |
| `POST upgrade.secu100.net/latestCacheMulti` | `["<device serial>"]` | `[]` (no firmware cached) |
| `POST boss22-api.xmcsrv.net/{bossBanner,bossServiceRecommendation,bossAlgorithmRecommendation}/list` | mall JSON, AES-wrapped `req` | mall payloads |

Two of these carry the account's e-mail address in clear text
(`promotion…campaign/get/info.username`), which is worth knowing before any of this
is proxied by us.

## 7. What this capture does and does not prove

Proved: host set; the identifiers above; the mall token exchange and its JWT claim
layout; the share-list envelope; the unauthenticated `asMsgQ` caps call and the
unauthenticated `appCaps` calls; that no call to `api.xmeye.net` happens at all.

Not proved *from this capture*: how the JWT is minted, the device-list call itself,
and any streaming or playback path. All three sit behind `pub-dscn.secu100.net`, which stayed tunnelled
while every other vendor host decrypted, i.e. pinning rather than a missing
proxying rule. Reaching it needs a jailbroken/hooked client or static work on the
Mach-O, not another proxy export.

Proved *since*, by running the vendor SDK out of tree (`protocol.md` section 3A)
and then reimplementing the envelope in Rust (3B): email + password yields the
`accessToken`, the platform exchange on `boss22-api.xmcsrv.net` answers
`code 2000`, login mode returns `msdlistCode`, and the signed envelope is
`POST /v3/<tsid>/<sig>.rs` with `tsid` = `0000003` + epoch ms, a 32-hex
per-request `sig`, a 96-hex body (AES over 12 random + 4-byte timestamp +
12-char seed) and no `UniqueCode` header. That `sig` is produced in
`crates/core/src/cloud/envelope.rs` and checked against the captured rows, so
`GET /api/devices` answers the account's real device list; the media path is
still out of reach.

## 8. What this project can use now

1. `GET /api/cloud/device-caps?sn=<serial>`, implemented in `cloud::app::CapsClient`
   and mounted in `crates/server/src/routes/cloud.rs`, backed by
   `caps.xmcsrv.net/api/asMsgQ`: model, firmware version, WAN address and capability
   block for a device we already know, with no account session. This is the one
   cloud call that needs no login. An empty serial answers
   `400`, and a serial the cloud does not know answers `404`: the vendor itself
   answers `ret 200` with blank `model` and `pid` for such a serial, so the route
   treats that as not found instead of passing blanks to the UI. `PROVIEW_CAPS_BASE`
   overrides the host.
2. `appCaps` as a cheap reachability and feature-flag probe for the cloud hosts.
3. The portal path of section 10 stays as it is: the app's login is a different
   mechanism and does not make the v2 portal captcha/login any more solved.
4. The cloud account login is part of the app: `POST /api/login` runs the
   signed v3 envelope from `crates/core/src/cloud/envelope.rs`, the device list
   follows on `GET /api/devices`, and the only configuration is the app identity
   (`PROVIEW_APP_UUID`/`PROVIEW_APP_KEY`/`PROVIEW_APP_SECRET`). The out-of-tree
   emulator of `protocol.md` section 3A is how the envelope was learned; it is
   not needed to log in.

