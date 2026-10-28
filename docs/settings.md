# Settings: every key in the app

The server reads its configuration from the environment, and from a settings file the
app writes for you, so nothing has to be exported by hand.

## Where it lives

`<data dir>/settings.json`, shaped as

```json
{ "values": { "PROVIEW_DEVICE_USER": "admin", "PROVIEW_PROVIDER": "local" } }
```

The data directory is the one the desktop app resolves for itself
(`%APPDATA%\\io.zhafron.proview-cctv` on Windows, `~/.local/share/io.zhafron.proview-cctv`
on Linux, `~/Library/Application Support/io.zhafron.proview-cctv` on macOS) or
`PROVIEW_DATA_DIR` for the standalone server. The resolved path is shown in the
Settings view and returned by `GET /api/settings`.

## How it applies

1. On start the file is read and every key is exported into the environment **only when
   that variable is not already set**. An exported variable therefore wins until it is
   cleared.
2. `POST /api/settings` writes the file atomically and exports the values into the
   process environment at once. Each save merges into what is already stored, so a second
   save never drops the first. Keys read per request (`PROVIEW_MODE`,
   `PROVIEW_PORTAL_BASE`, `PROVIEW_CAPS_BASE`, `PROVIEW_CLOUD_SESSION_TTL`,
   `PROVIEW_FFMPEG`, `PROVIEW_LENS_PTZ`, `PROVIEW_LENS_CAPABILITIES`) apply
   immediately. Keys marked *needs restart* are read once at startup — provider, cloud
   base, device host/RTSP/ONVIF/user/password, session lifetime, fixture directory,
   keychain profile, listen address, data directory and cookie secret — and the view
   offers a **Restart the app** button for the desktop build. That button posts to
   `POST /api/desktop/restart` with the per-run token the shell writes into the
   served page (`window.__PROVIEW_DESKTOP__`), and the shell calls `app.restart()`;
   the standalone server mounts no such route and its view says to restart the
   process instead.
3. Secret values (`PROVIEW_DEVICE_PASS`, `ROCKET_SECRET_KEY`) are never returned: the
   API reports `set: true` and `value: null`. An empty input leaves a stored secret
   alone; sending an empty string for a non-secret key clears it.
4. The view opens from the sign-in screen (`Settings` under the form), so the cloud
   address, the camera address and the camera password can be set before anyone can
   sign in. In that mode the top-left button reads `Sign in` instead of `Live` and
   returns to the form; the Connection section asks for a session before the
   provider can be switched.
5. `ROCKET_SECRET_KEY` is the one value a release build cannot take from the
   file on a first run: Rocket refuses to boot without it, so set it once in the
   environment (or in `settings.json` before the first start). Afterwards the
   view shows it as set and lets you replace it.
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

Each key carries a plain label and one sentence of hint, and the two select keys
show words rather than identifiers:

| Key | Label | Options shown |
|---|---|---|
| `PROVIEW_PROVIDER` | Provider | Vendor cloud, This network, Built-in sample data |
| `PROVIEW_MODE` | Default mode | Cloud account, This network, Built-in sample data |

Labels are otherwise the thing itself — "Cloud address", "Camera address", "Camera
video address", "Camera control address", "Camera user", "Camera password",
"Sign-in lifetime", "Server address", "ffmpeg program", "Movement channel", "Lens
features", "Sample data folder", "Credential profile", "Data folder", "Sign-in
cookie secret". No message the app shows names a `PROVIEW_*` variable; a missing
value says which screen owns it.

## Keys

| Group | Key | Notes |
|---|---|---|
| Cloud | `PROVIEW_PROVIDER` | `proview`, `local` or `fixture` |
| Cloud | `PROVIEW_MODE` | default mode for a session that has not chosen one |
| Cloud | `PROVIEW_CLOUD_BASE` | host for the signed cloud login envelope (default `https://rs.xmeye.net`) |
| Cloud | `PROVIEW_PORTAL_BASE` | host for the portal account login |
| Self-built builds | `PROVIEW_APP_UUID`, `PROVIEW_APP_KEY`, `PROVIEW_APP_SECRET` | the app's platform identity for the signed envelope; a release build bakes it in, so these are only for a build made from source without one |
| Cloud | `PROVIEW_CAPS_BASE` | host for device capabilities by serial |
| Cloud | `PROVIEW_CLOUD_SESSION_TTL` | seconds a cloud session is kept |
| Local network | `PROVIEW_DEVICE_HOST` | DVRIP `host:port` |
| Local network | `PROVIEW_DEVICE_RTSP` | RTSP `host:port` |
| Local network | `PROVIEW_DEVICE_ONVIF` | ONVIF base address |
| Local network | `PROVIEW_DEVICE_USER` | camera user |
| Local network | `PROVIEW_DEVICE_PASS` | camera password, secret |
| Server | `PROVIEW_SESSION_TTL` | browser session lifetime in seconds |
| Server | `PROVIEW_LISTEN` | standalone server bind address |
| Streaming | `PROVIEW_FFMPEG` | ffmpeg binary; empty means `ffmpeg` from `PATH` |
| Streaming | `PROVIEW_LENS_PTZ` | channel that carries movement |
| Streaming | `PROVIEW_LENS_CAPABILITIES` | comma list of lens capabilities to advertise |
| Advanced | `PROVIEW_FIXTURE_DIR` | directory the fixture provider reads |
| Advanced | `PROVIEW_PROFILE` | keychain profile used by the desktop |
| Advanced | `PROVIEW_DATA_DIR` | data directory, restart |
| Advanced | `ROCKET_SECRET_KEY` | cookie signing secret, secret, restart |

Unknown keys are rejected, numbers must parse, and a select must be one of its options;
each failure comes back as `400 {"ok":false,"message":"…"}`. `GET /api/settings` also
reports `exists` so the view can say whether anything has been saved yet. A missing
camera address answers `400 {"ok":false,"code":"device_host_missing",…}` (and
`device_rtsp_missing` for the video address), so the UI keeps its guidance out of the
message text. Every message the app shows names the Settings screen instead of the key
it writes behind it.

## API

| Call | Body | Reply |
|---|---|---|
| `GET /api/settings` | — | `{path, exists, fields:[{key,label,group,kind,options,hint,restart,secret,set,value}]}` |
| `POST /api/settings` | `{"values":{ "<KEY>": "<value>" }}` | the same view, after the file is written |
