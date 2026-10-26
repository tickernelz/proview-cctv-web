# proview-cctv-web

A small Rocket server that puts PROVIEW CCTV (package `com.proview.cctv`, a
Xiongmai / XMEye white-label) cameras on a desktop browser instead of a phone.

Status: **protocol reconnaissance, skeleton server**. The web layer, session
handling and provider boundary are real; the vendor cloud calls and the live
video transport are not finished. See `docs/protocol.md` for exactly what is
proven and what is not.

## What works today

| Piece | State |
|---|---|
| Rocket server, static UI, JSON API | works |
| Server-side session (private cookie → in-memory vendor session) | works |
| Fixture provider (device list, stream ticket) | works, default |
| Vendor cloud login | partially mapped, returns `501` until the envelope is solved |
| Device list from the vendor cloud | not implemented |
| Live video to the browser | not implemented |

The server never sends a vendor token to the browser: the browser holds only an
opaque `pvw_session` cookie, and every vendor call happens server side.

## Run

```sh
cargo run --release
# http://127.0.0.1:8000
```

Fixture mode is the default and needs no account:

```
email:    fixture@example.invalid
password: fixture-password
```

To point it at the real cloud:

```sh
export PROVIEW_PROVIDER=proview
export PROVIEW_CLOUD_BASE=https://pub-token.xmeye.net
cargo run --release
```

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `PROVIEW_PROVIDER` | `fixture` | `fixture` or `proview` |
| `PROVIEW_CLOUD_BASE` | `https://pub-token.xmeye.net` | vendor cloud base |
| `PROVIEW_DEVICE_USER` | `admin` | device-side account for RTSP |
| `PROVIEW_DEVICE_PASS` | empty | device-side password |
| `PROVIEW_FIXTURE_DIR` | `fixtures` | fixture data directory |
| `PROVIEW_SESSION_TTL` | `3600` | server session lifetime, seconds |

Nothing here is a vendor secret. The extracted application credentials from the
APK are deliberately **not** in this repository; `docs/protocol.md` records
where they come from and what they are used for, not their values.

## Layout

```
src/config.rs        env configuration
src/session.rs       opaque session id → vendor session, in memory
src/provider/        the seam: fixture | proview
src/routes.rs        JSON API + static files
static/              UI (no build step)
fixtures/            recorded device metadata for local runs
docs/protocol.md     the reverse-engineering record
```

## Licence

MIT, see `LICENSE`.

This project ships no vendor code, no APK, no vendor `.so` and no extracted
keys. It talks to a device and a cloud account you own.
