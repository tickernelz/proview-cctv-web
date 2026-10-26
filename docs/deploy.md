# Deploying the server

The binary speaks plain HTTP on the loopback interface by default. Terminate
TLS in front of it; no TLS code lives in this repository.

## Local

```sh
ROCKET_SECRET_KEY="$(openssl rand -base64 32)" ./proview-server --listen 127.0.0.1:8787
```

`ROCKET_SECRET_KEY` must be stable across restarts (it signs the session
cookie) and at least 44 base64 characters. Generate once, store in your
secret manager.

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

- The server holds vendor credentials in memory only; they are never written to
  disk by the server binary.
- The desktop build binds an ephemeral loopback port with a per-launch secret
  key, so a local attacker cannot reuse a cookie across launches.
- There is no multi-user support yet: one server instance serves one vendor
  account.
