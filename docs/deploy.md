# Deploying the server

The binary speaks plain HTTP on the loopback interface by default. Terminate
TLS in front of it; no TLS code lives in this repository.

## Local

```sh
ROCKET_SECRET_KEY="$(openssl rand -base64 32)" ./proview-server --listen 127.0.0.1:8787
```

`ROCKET_SECRET_KEY` must be stable across restarts (it signs the session
cookie). Rocket accepts exactly 32 random bytes: 44 base64 characters
(`openssl rand -base64 32`) or 64 hex characters, and refuses anything else with
`InsecureSecretKey` / `InvalidValue`. A release build will not start without
it. Generate once, store it in your secret manager.

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

The settings file, the mode store and the desktop log live in the data
directory (`PROVIEW_DATA_DIR`, `StateDirectory` above makes systemd create it
with the right owner). Everything else the operator can set from Settings in the
browser; the environment variables in the unit win over the file until the value
is cleared in the view.

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
ExecStart=/usr/local/bin/proview-server --listen 127.0.0.1:8787
Restart=on-failure
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true

[Install]
WantedBy=multi-user.target
```

## Threat model

- The server keeps the vendor account session in memory only. Camera
  credentials and `ROCKET_SECRET_KEY` are written to disk **only** when the
  operator saves them in Settings, and then they live in `settings.json` in the
  data directory. Treat that directory as a secret: create it 0700 and let the
  service user own it.
- Secrets are never returned by the API: the view reports that a value is set,
  not what it is.
- The desktop build binds an ephemeral loopback port with a per-launch secret
  key, so a local attacker cannot reuse a cookie across launches.
- There is no multi-user support yet: one server instance serves one vendor
  account.
