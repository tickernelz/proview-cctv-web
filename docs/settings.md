# Settings: every key in the app

The server reads its configuration from the environment, and from a settings file the
app writes for you, so nothing has to be exported by hand.

## Where it lives

`<data dir>/settings.json`, shaped as

```json
{ "values": { "PROVIEW_DEVICE_USER": "admin", "PROVIEW_MODE": "local" } }
```

The same file keeps the live grid layout under `grid`; a settings save leaves it alone.

The desktop app uses the data directory it resolves for itself
(`%APPDATA%\io.zhafron.proview-cctv` on Windows, `~/.local/share/io.zhafron.proview-cctv`
on Linux, `~/Library/Application Support/io.zhafron.proview-cctv` on macOS). The
standalone server has no default: it uses `PROVIEW_DATA_DIR` from its environment, and
without it nothing is read or written, `GET /api/settings` answers `path: null`, and a
save changes only the running process until it exits. The resolved path is shown in
the Settings view and returned by `GET /api/settings`.

## Who may read or write it

The route carries the camera credentials, the cloud address and the cookie secret,
so both calls are gated. The desktop build answers only on the authority the shell
bound (`127.0.0.1:<port>`) and only to a request that holds a live sign-in or the
per-run page token from `window.__PROVIEW_DESKTOP__`; anything else gets `403`
(another `Host`) or `401` (no session and no token). The standalone server answers
only a request that **arrived from this machine** - a loopback TCP peer, read from
the socket rather than from a header - on a loopback `Host` (`localhost`, `[::1]` or
a `127.x.x.x` address, with or without a port), and refuses every other peer or
`Host` with `403`, so neither the network nor a name that resolves to this machine
through DNS rebinding can read or write it, while a browser page on this machine
keeps the pre-sign-in path the sign-in screen needs. Over a direct connection that
peer cannot be forged by a client on the network; a forwarder on this machine (a
reverse proxy, `ssh -L`, `socat`) gives every client it carries a loopback peer, so
keep `/api/settings` out of such a forwarder. A request that carries an `Origin`
must carry this server's own origin.

## How it applies

1. On start the file is read and every known key except the `PROVIEW_DESKTOP_*` ones
   is exported into the environment **only when that variable is not already set**.
   An exported variable therefore wins until it is cleared. The standalone server
   takes its listen address from `--listen`, then `PROVIEW_LISTEN` in the
   environment, then the value saved in `settings.json`, then `127.0.0.1:8787`,
   and logs the address it chose with its source. Each must be an IP address and
   port (`127.0.0.1:8787`, `[::1]:8787`; names such as `localhost` are not
   resolved): Settings refuses any other value with `400` and writes nothing, and a
   bad `--listen` or environment value stops the start. A saved value is used only
   when it is a loopback address. A saved network address (`0.0.0.0:8787`, a LAN
   address) is ignored at start with a warning naming `settings.json`, so exposing
   the server to the network always takes `--listen` or the environment.
2. `POST /api/settings` writes the file atomically (staged `settings.json.tmp`, then
   a rename) and exports the values into the process environment at once. Each save
   merges into what is already stored, so a second save never drops the first. Keys
   read when they are used (`PROVIEW_FFMPEG`, `PROVIEW_LENS_PTZ`,
   `PROVIEW_LENS_CAPABILITIES`, `PROVIEW_DEVICE_SERIAL`) apply immediately. Keys
   with the **Restart** tag (the `Restart` column below) are read once at startup, and
   the view offers a **Restart the app** button for the desktop build. That includes
   `PROVIEW_MODE` (the mode store's default) and `PROVIEW_PORTAL_BASE`,
   `PROVIEW_CAPS_BASE`, `PROVIEW_CLOUD_SESSION_TTL` (the cloud state built once at
   startup). The `PROVIEW_APP_*` identity is resolved per cloud
   call, except in the clients that cloud state builds at startup. The restart button
   posts to `POST /api/desktop/restart` with the per-run token the shell writes into
   the served page (`window.__PROVIEW_DESKTOP__`), and the shell calls
   `app.restart()`; on the standalone server that route answers `404` and the view
   says to restart the process instead. Both the route and the page answer only the
   authority the shell bound (`127.0.0.1:<port>`): a request whose `Host` is anything
   else, or whose token does not match, gets a 403, and `GET /` serves the page
   without the token.
3. Secret values (`PROVIEW_DEVICE_PASS`, `ROCKET_SECRET_KEY`, `PROVIEW_APP_SECRET`)
   are never returned: the API reports `set: true` and `value: null`. The view never
   sends an empty secret input, so leaving it empty keeps the stored secret; an empty
   string sent for any key clears it.
4. The view opens from the sign-in screen (the **Open Settings** button), so the cloud
   address, the camera address and the camera password can be set before anyone can
   sign in. In that mode the top-left button reads `Sign in` instead of `Live` and
   returns to the form; the Connection section asks for a session before the
   provider can be switched.
5. `ROCKET_SECRET_KEY` is the one value a release build of the standalone server
   cannot take from the file on a first run: Rocket refuses to boot without it, so
   set it once in the environment (or in `settings.json` before the first start).
   Afterwards the view shows it as set and lets you replace it. The desktop build
   ignores it and signs the cookie with a key drawn fresh on every run, so a sign-in
   does not survive a restart; its Settings view does not list the key and refuses a
   save that carries it.
6. The file is plain JSON, so its directory is the protection: create the data
   directory 0700 and let only the service user own it. The desktop build keeps the
   interactive camera login in the OS keychain instead; there the order is keychain,
   then environment, then `settings.json`.

7. The cloud app identity is not something an operator fills in: a release build
   carries `PROVIEW_APP_UUID`, `PROVIEW_APP_KEY` and `PROVIEW_APP_SECRET` as
   constants the build baked in from the release workflow's secrets. The environment
   still wins per value when it is set, which is how a build made from source takes
   its own identity. Rotating the identity is a vendor change followed by a release:
   an installed binary keeps the identity it was built with.

## What the view shows

Each key carries the plain label of the table below and a one-sentence hint, and the
two select keys show words rather than identifiers:

| Key | Label | Options shown |
|---|---|---|
| `PROVIEW_PROVIDER` | Provider | ProView camera, Built-in sample data |
| `PROVIEW_MODE` | Default mode | Cloud account, This network |

The `Desktop` group is shown only inside the desktop app. Validation and
missing-value messages name the field's label or the Settings screen that owns the
value, not the variable.

## Keys

The `FIELDS` table in `crates/server/src/settings.rs`, in order. *Default* is what the
server does with an unset key; only the four `Desktop` keys report it in the API's
`default`.

| Group | Key | Label | Kind | Restart | Default and notes |
|---|---|---|---|---|---|
| Cloud | `PROVIEW_PROVIDER` | Provider | select `proview`, `fixture` | yes | `proview`, the camera through the cloud or the local mode; `fixture` selects the built-in sample data. Any other stored value counts as `proview`. The LAN path is the mode, not this key |
| Cloud | `PROVIEW_MODE` | Default mode | select `cloud`, `local` | yes | `cloud`. Read at startup; a `mode.json` in the data directory wins over it |
| Cloud | `PROVIEW_CLOUD_BASE` | Cloud address | text | yes | `https://rs.xmeye.net`, the signed cloud login envelope |
| Cloud | `PROVIEW_PORTAL_BASE` | Portal address | text | yes | `https://api.xmeye.net`, the account login |
| Cloud | `PROVIEW_CAPS_BASE` | Capability address | text | yes | `https://caps.xmcsrv.net`, device capabilities by serial |
| Cloud | `PROVIEW_CLOUD_SESSION_TTL` | Cloud session lifetime | number | yes | `3600` seconds |
| Local network | `PROVIEW_DEVICE_HOST` | Camera address | text | yes | unset; DVRIP `host:port` |
| Local network | `PROVIEW_DEVICE_RTSP` | Camera video address | text | yes | unset; RTSP `host:port`, port `554` when omitted |
| Local network | `PROVIEW_DEVICE_ONVIF` | Camera control address | text | yes | unset; ONVIF base address |
| Local network | `PROVIEW_DEVICE_SERIAL` | Camera serial for the device sign-in | text | yes | unset; the serial the camera user and password belong to. Read when used |
| Local network | `PROVIEW_DEVICE_USER` | Camera user | text | yes | `admin` |
| Local network | `PROVIEW_DEVICE_PASS` | Camera password | secret | yes | empty |
| Server | `PROVIEW_SESSION_TTL` | Sign-in lifetime | number | yes | `3600` seconds |
| Server | `PROVIEW_LISTEN` | Server address | text | yes | `127.0.0.1:8787`; standalone server only: `--listen`, then the environment, then this saved value when it is loopback; an IP address and port (see above) |
| Streaming | `PROVIEW_FFMPEG` | ffmpeg program | path | no | `ffmpeg` from `PATH` |
| Streaming | `PROVIEW_LENS_PTZ` | Movement channel | number | no | unset; the 0-based lens index that moves, see [the lens contract](contracts/lens.md) |
| Streaming | `PROVIEW_LENS_CAPABILITIES` | Lens features | text | no | empty; comma list of lens capabilities to advertise |
| Advanced | `PROVIEW_FIXTURE_DIR` | Sample data folder | path | yes | `fixtures` in the working directory, else beside the executable, else the source checkout's |
| Advanced | `PROVIEW_PROFILE` | Credential profile | text | yes | `default`; the desktop keychain profile |
| Advanced | `PROVIEW_DATA_DIR` | Data folder | path | yes | standalone server: none (see above); the desktop app uses its own |
| Advanced | `ROCKET_SECRET_KEY` | Sign-in cookie secret | secret | yes | none; not listed in the desktop app; see 5 above |
| Self-built builds | `PROVIEW_APP_UUID` | Platform app uuid | text | no | the baked identity; only for a build made from source without one |
| Self-built builds | `PROVIEW_APP_KEY` | Platform app key | text | no | as above |
| Self-built builds | `PROVIEW_APP_SECRET` | Platform app secret | secret | no | as above |
| Desktop | `PROVIEW_DESKTOP_ALARM_NOTIFY` | Alarm notifications | toggle | no | `off`; a native system notification for each new alarm |
| Desktop | `PROVIEW_DESKTOP_ALARM_INTERVAL` | Alarm check interval | number, 15 to 3600 | no | `60` seconds between alarm checks |
| Desktop | `PROVIEW_DESKTOP_CLOSE_TO_TRAY` | Keep running when the window closes | toggle | no | `on`; closing the window leaves the app in the tray |
| Desktop | `PROVIEW_DESKTOP_MINIMIZE_TO_TRAY` | Hide to tray when minimized | toggle | no | `off`; minimizing hides the window to the tray |

The four `PROVIEW_DESKTOP_*` keys are read by the desktop shell, never exported into
the environment, and take effect on the next alarm check or window event; a build
without the desktop shell simply ignores them. `docs/desktop-notifications.md` says
what each one does to the window and to the notifications.

Unknown keys are rejected, a number must be a whole number, a toggle must be `on` or
`off`, a number with bounds must sit inside them, and a select must be one of its
options; an empty value skips the checks and clears the key. Each failure comes back as
`400 {"ok":false,"message":"…"}`. `GET /api/settings` also reports `exists` so the
view can say whether anything has been saved yet. A probe without a camera address
answers `400 {"ok":false,"code":"device_host_missing",…}` (`POST /api/device/probe`;
`device_rtsp_missing` from `POST /api/device/rtsp` for the video address), so the UI
keeps its guidance out of the message text.

## Environment only

These are read by the server but are not settings: they are not in the view,
`POST /api/settings` rejects them and `settings.json` cannot carry them. A timing value
that does not parse, or is `0`, falls back to the default.

| Variable | Default | What it bounds or points at |
|---|---|---|
| `PROVIEW_DVRIP_WINDOW_MS` | `20000` | each DVRIP operation on a camera |
| `PROVIEW_SETTINGS_WINDOW_MS` | `20000` | camera settings reads and writes (settings, detection, image routes) |
| `PROVIEW_FORMAT_WINDOW_MS` | `60000` | a storage format |
| `PROVIEW_SETTINGS_BACKUP_DIR` | `<PROVIEW_DATA_DIR or working dir>/settings-backups` | where the camera's config is copied before a settings write |
| `PROVIEW_PLAYBACK_WINDOW_MS` | `20000` | playback calendar and thumbnails |
| `PROVIEW_ACCOUNT_WINDOW_MS` | `15000` | cloud account calls |
| `PROVIEW_SHARE_WINDOW_MS` | `12000` | device sharing calls |
| `PROVIEW_SYSTEM_WINDOW_MS` | `20000` | camera system calls |
| `PROVIEW_SYSTEM_NETWORK_TOTAL_MS` | `60000` | the whole network check |
| `PROVIEW_CLOUD_CLIP_WINDOW_MS` | `7000` | each cloud recording call (values under 50 ignored) |
| `PROVIEW_CLOUD_CLIP_TOTAL_MS` | `120000` | a whole cloud clip download (values under 50 ignored) |
| `PROVIEW_ALARM_WINDOW_MS` | `8000` | each vendor call of the alarm routes |
| `PROVIEW_ALARM_POLL_SECONDS` | `20` | the poll interval the alarm list tells the page (values under 5 ignored) |
| `PROVIEW_PTZ_HOLD_MS` | `30000` | the longest continuous move before the server stops it |
| `PROVIEW_ALARM_HOST`, `PROVIEW_ALARM_PORT` | `access-alc.secu100.net`, `6602` | vendor alarm server |
| `PROVIEW_CSS_HOST` | `access-css.secu100.net` (port `6614` unless `host:port`) | vendor cloud storage server |
| `PROVIEW_PMS_BASE` | `http://access-pms.secu100.net:6602` | vendor server the cloud recording search posts to |
| `PROVIEW_UPGRADE_BASE` | `https://upgrade.secu100.net:9083` | firmware upgrade check |
| `PROVIEW_SHARE_BASE` | `https://as-ams.jftechws.com` | device sharing service |
| `PROVIEW_DEVICE_MODULUS` | unset | RSA modulus that seals the cloud device login when the device sends no public key |

## API

| Call | Body | Reply |
|---|---|---|
| `GET /api/settings` | — | `{path, exists, fields:[{key,label,group,kind,options,hint,restart,secret,set,value,default,min,max}]}`; `kind` is `text`, `secret`, `number`, `select`, `toggle` or `path`, `default` is what an unset key behaves as (`Desktop` keys only), `min`/`max` bound a number |
| `POST /api/settings` | `{"values":{ "<KEY>": "<value>" }}` | the same view, after the file is written |
| either call from another `Host`, or from an `Origin` that is not this server's own | — | `403 {"ok":false,"message":"…"}`; in the desktop build a request with no session and no page token answers `401` |
