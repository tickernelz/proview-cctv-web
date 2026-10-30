# Documentation

## Use it

| Doc | For |
|---|---|
| [install.md](install.md) | Installing the desktop app and the server binary on each platform, first launch of unsigned builds |
| [deploy.md](deploy.md) | Running `proview-server` behind Caddy with systemd, the cloud identity in a release, threat model |
| [settings.md](settings.md) | Every configuration key, its Settings label, defaults, precedence and the settings API |
| [providers.md](providers.md) | The vendor cloud, a camera on the LAN, and the fixture provider |
| [desktop-notifications.md](desktop-notifications.md) | Alarm notifications, tray, close and minimize to tray |
| [webview-matrix.md](webview-matrix.md) | Which webview decodes H.265 natively and when ffmpeg is needed |
| [parity.md](parity.md) | Every feature of the phone app, its state here, and the remaining boundaries |

## How it works

| Doc | For |
|---|---|
| [protocol.md](protocol.md) | The protocol record: cloud envelope, relay, media cipher, DVRIP, RTSP, ONVIF, account calls |
| [cloud-app-surface.md](cloud-app-surface.md) | What the iOS app talks to in the vendor cloud (jftechws, xmcsrv, secu100), from a capture |
| [contracts/live.md](contracts/live.md) | Live stream contract between server and page |
| [contracts/lens.md](contracts/lens.md) | Dual-lens split contract (`GET /api/lens`) |
| [fisheye-model.md](fisheye-model.md) | Fisheye dewarp projection model recovered from the vendor library |
| [apmode.md](apmode.md) | Adding a camera over AP mode: what the app sends, recovered from the binary |
| [adr/0001-frontend-stack.md](adr/0001-frontend-stack.md) | Decision: Vue 3 + TypeScript + Vite + Tailwind |
| [adr/0002-ui-library.md](adr/0002-ui-library.md) | Decision: UI component layer (reka-ui) and the v2 theme |

## Measurements and acceptance

| Doc | For |
|---|---|
| [performance.md](performance.md) | Measured latency, memory and sign-in timings |
| [live-acceptance.md](live-acceptance.md) | Acceptance runs against the real camera over the relay |
| [mobile-lab.md](mobile-lab.md) | Running the shipped arm64 app on an emulator in CI |

Also: [../CHANGELOG.md](../CHANGELOG.md), [../e2e/README.md](../e2e/README.md)
(Playwright suite), [../fixtures/README.md](../fixtures/README.md) (sample data).
