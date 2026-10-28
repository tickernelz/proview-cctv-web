# End-to-end tests

Playwright (`@playwright/test` 1.64.1, pinned exactly) drives the page that
`proview-server` serves from `apps/web/dist`, against the fixture provider
(no camera, no vendor account), in Chromium and WebKit. No test calls a model.

- `global-setup.ts` starts one `proview-server` per scenario in
  `scenarios.mjs` (empty, one, many cameras; per-browser copies for the specs
  that save settings or the grid layout, so test order cannot leak between
  specs) and stops them after the run.
- `tests/*.spec.ts` find elements by `data-testid` only.
- `tests/visual.spec.ts` compares six screens at 1440x900 and 1280x800 in the
  dark and the light theme against `tests/visual.spec.ts-snapshots/`
  (`maxDiffPixelRatio` 0.002; live video and clocks masked).
- Retry 1; a failed test keeps its trace and video in `test-results/`.

## Run it locally

```sh
(cd apps/web && npm ci && npm run build)
OPENSSL_NO_VENDOR=1 ~/re/proview-cctv/build-slot.sh cargo build -p proview-server
cd e2e
npm ci
npm run serve &        # fixture servers on the host; Ctrl-C stops them all
./run.sh               # the suite in mcr.microsoft.com/playwright, as CI renders it
./run.sh --project=chromium tests/live.spec.ts
```

`run.sh` runs Playwright in the image that matches the pinned version, so
screenshots render with the same fonts as CI, and reuses the host servers
(`PROVIEW_E2E_REUSE=1`) because a binary linked on the host may not run inside
the image. In CI, `npx playwright test` starts the servers itself with the
binary built in the same container.

Set `PROVIEW_SERVER_BIN` when the binary is not `target/debug/proview-server`.

## Update the screenshots

```sh
./run.sh --update-snapshots
```

Look at every changed PNG before committing it, and commit new baselines in
the same commit as the change that moved the pixels.

## Why Playwright and not tester-army/e2e

Both ran the same 84 tests (2 workers, retry 1, same image and servers) on
2026-10-28, three rounds each, servers restarted before each cold run:

| Harness | Cold | Warm | Median |
|---|---|---|---|
| `e2e` 0.21.3 + `@e2e-dev/web` 0.15.3, `--strict-cache` | 76 / 64 / 54 s | 70 / 72 / 76 s | 71 s |
| `@playwright/test` 1.64.1 | 45 / 43 / 50 s | 44 / 42 / 46 s | 44.5 s |

No test needed an agent step, which is the part `e2e` adds on top of
Playwright, so Zhafron chose plain Playwright (2026-10-28 15:59Z).
