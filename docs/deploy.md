# Deploying the server

The binary speaks plain HTTP on the loopback interface by default. Terminate
TLS in front of it; no TLS code lives in this repository.

## Local

```sh
ROCKET_SECRET_KEY="$(openssl rand -base64 32)" ./proview-server --listen 127.0.0.1:8787
```

`ROCKET_SECRET_KEY` must be stable across restarts (it signs the session
cookie). Rocket takes a 256-bit key as 44 base64 characters
(`openssl rand -base64 32`) or 64 hex characters (a 512-bit key as 88 base64
characters also works); a malformed value fails the start with a configuration
error (`InvalidValue` / `InvalidLength`). A release build will not start
without it (`InsecureSecretKey`). Generate once, store it in your secret
manager.

`--listen` (or `PROVIEW_LISTEN` in the environment) sets the address; the
default is `127.0.0.1:8787`. Set `PROVIEW_DATA_DIR` too: the server has no
default data directory, and without one the values saved in Settings last only
until the process exits.

## Caddy (recommended)

`/etc/caddy/Caddyfile`:

```
cctv.example.com {
    encode zstd gzip
    reverse_proxy 127.0.0.1:8787 {
        header_up X-Forwarded-Proto {scheme}
    }
}
```

Caddy obtains and renews the certificate automatically. Nothing else is needed:
the app already sets `HttpOnly` + `SameSite=Strict` cookies, and because the
proxy is same-origin the cookie keeps working without extra configuration.

## systemd

```ini
[Unit]
Description=PROVIEW CCTV web
After=network-online.target

[Service]
User=cctv
Environment=ROCKET_SECRET_KEY=<from your secret manager>
Environment=PROVIEW_DATA_DIR=/var/lib/proview-cctv
StateDirectory=proview-cctv
StateDirectoryMode=0700
ExecStart=/usr/local/bin/proview-server --listen 127.0.0.1:8787
Restart=on-failure
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true

[Install]
WantedBy=multi-user.target
```

The data directory (`PROVIEW_DATA_DIR`) holds the settings file
(`settings.json`), the mode store (`mode.json`), the remembered account
(`account.json`), the device token store (`device-tokens.json`, encrypted under
`device-token.key`; both written automatically after a cloud login) and the
copies of the camera's settings taken before each settings write
(`settings-backups/`, unless `PROVIEW_SETTINGS_BACKUP_DIR` points elsewhere;
without a data directory they go to `./settings-backups`). `StateDirectory`
above makes systemd create it, owned by the service user and mode 0700. Everything else the operator can set from
Settings, but the settings answer only a request from the machine itself to a
loopback address, so they refuse through the proxy; open them over an SSH tunnel
(`ssh -L 8787:127.0.0.1:8787 <host>`, then <http://127.0.0.1:8787>). A variable
set in the unit wins over `settings.json` at every start, so a value saved in
the view for a key the unit also sets is lost at the next restart.

## Cloud identity in a release

The vendor cloud sign-in signs with the app's platform identity: an app uuid, an
app key and an app secret. No value of it is in this repository. The `release`
workflow reads it from the repository secrets `PROVIEW_APP_UUID`,
`PROVIEW_APP_KEY` and `PROVIEW_APP_SECRET`, writes them to a
`cloud-identity.env` file in the workspace root and sets
`PROVIEW_REQUIRE_CLOUD_IDENTITY=1`; `crates/core/build.rs` bakes the three
values into the binary as constants, logs one line that the identity is present
(never the values), and fails the build when any of them is missing. Before
packaging, every job also greps the built binary for the three values, so a
target that somehow skipped the injection fails the release instead of shipping
without an identity. The file is
gitignored, exists only on the runner, and the release jobs keep the target
directory out of the Actions cache so the values are not cached either.

A build made from these sources without that step has no identity. It still
serves the local network and the sample provider; the cloud sign-in answers that
the build was made without a cloud identity, and points at
`docs/cloud-app-surface.md`, instead of asking anyone to set variables.

Building from source with an identity of your own:

```sh
(cd apps/web && npm ci --include=dev && npm run build)
PROVIEW_REQUIRE_CLOUD_IDENTITY=1 \
PROVIEW_APP_UUID=... PROVIEW_APP_KEY=... PROVIEW_APP_SECRET=... \
cargo build --release -p proview-server
```

The file is only watched once it exists: after creating or replacing
`cloud-identity.env`, run `touch crates/core/build.rs` (or set
`PROVIEW_REQUIRE_CLOUD_IDENTITY=1`) so the build script runs again. A build
configured while the file was absent keeps the identity it baked, which is none.
`Cross.toml` passes `PROVIEW_REQUIRE_CLOUD_IDENTITY` into the cross containers,
so the cross targets fail on a missing identity instead of shipping without one.

At run time the environment still wins per value: `PROVIEW_APP_UUID`,
`PROVIEW_APP_KEY` or `PROVIEW_APP_SECRET` set in the service environment
override what the build baked in.

Rotating the identity is a release, not a configuration change. The vendor issues
a new one, the three repository secrets are replaced, and every installed copy
needs a binary built after that: a release keeps the identity it was built with
forever, so an old installer cannot be repaired from this side.

## Threat model

- The server keeps the vendor account session in memory, unless the user turns
  on **Remember this account in this browser** at sign-in: then the vendor
  access token is written to `account.json` in the data directory (0600),
  bound to a 30-day `pvw_remember` cookie in that browser, until sign-out
  clears it. Camera credentials and `ROCKET_SECRET_KEY` reach `settings.json`
  only when the operator saves them in Settings. Without any action, the device
  token a cloud login succeeds with is written to `device-tokens.json`
  (AES-256-GCM under `device-token.key`, beside it), and every camera settings
  write first copies the camera's current settings to `settings-backups/`.
  Treat the whole directory as a secret: create it 0700 and let the service
  user own it.
- Secrets are never returned by the API: the view reports that a value is set,
  not what it is.
- The desktop build binds an ephemeral loopback port with a per-launch secret
  key, so a local attacker cannot reuse a cookie across launches.
- There is no multi-user support yet: the server has no users or roles of its
  own, the settings are shared by every browser, and the data directory
  remembers one vendor account.
