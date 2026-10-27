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
   offers a **Restart the app** button for the desktop build.
3. Secret values (`PROVIEW_DEVICE_PASS`, `ROCKET_SECRET_KEY`) are never returned: the
   API reports `set: true` and `value: null`. An empty input leaves a stored secret
   alone; sending an empty string for a non-secret key clears it.
4. The view opens from the sign-in screen (`Settings` under the form), so the cloud host,
   the camera address and the camera password can be set before anyone can sign in. In
   that mode the Live and Grid tabs stay hidden until a session exists.

## Keys

| Group | Key | Notes |
|---|---|---|
| Cloud | `PROVIEW_PROVIDER` | `proview`, `local` or `fixture` |
| Cloud | `PROVIEW_MODE` | default mode for a session that has not chosen one |
| Cloud | `PROVIEW_CLOUD_BASE` | host for the app cloud API |
| Cloud | `PROVIEW_PORTAL_BASE` | host for the portal account login |
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
each failure comes back as `400 {"ok":false,"message":"…"}`.

## API

| Call | Body | Reply |
|---|---|---|
| `GET /api/settings` | — | `{path, fields:[{key,label,group,kind,options,hint,restart,secret,set,value}]}` |
| `POST /api/settings` | `{"values":{ "<KEY>": "<value>" }}` | the same view, after the file is written |
