# End-to-end tests

The suite drives the page that `proview-server` serves from `apps/web/dist`,
against the fixture provider (no camera, no vendor account), in Chromium and
WebKit, with [e2e](https://github.com/tester-army/e2e) (`e2e` 0.21.3 and
`@e2e-dev/web` 0.15.3, Apache-2.0, pinned exactly).

- `serve.mjs` starts one `proview-server` per scenario in `scenarios.mjs`
  (empty, one, many cameras; per-browser copies for the specs that save state)
  and writes their URLs to `.servers.json`. The config refuses to load without
  that file.
- `tests/*.e2e.ts` find elements by `data-testid` only.
- `tests/visual.e2e.ts` compares six screens at 1440x900 and 1280x800 in the
  dark and the light theme against `tests/visual.e2e.ts-snapshots/`.
- No test calls a model. A future `agent.act` step is recorded once with a
  model and committed under `.e2e/cache`; CI runs `--strict-cache`, so a
  stale recording fails with `REPLAY_STALE` instead of calling a model.

## Run it locally

```sh
(cd apps/web && npm ci && npm run build)
OPENSSL_NO_VENDOR=1 ~/re/proview-cctv/build-slot.sh cargo build -p proview-server
cd e2e
npm ci
npm run serve &        # keep it running; Ctrl-C stops every server
./run.sh               # the suite inside mcr.microsoft.com/playwright, as CI renders
./run.sh --target chromium tests/live.e2e.ts
```

`run.sh` runs the CLI in the Playwright image that matches the pinned
`playwright-core`, so screenshots render with the same fonts as CI. Running
`npx e2e run` on the host works for the functional specs, but the visual
specs then compare host rendering against the committed Linux baselines.

Set `PROVIEW_SERVER_BIN` when the binary is not `target/debug/proview-server`
(on this box: `$HOME/.cache/zha4181/target/debug/proview-server` with
`LD_LIBRARY_PATH=/home/linuxbrew/.linuxbrew/lib`).

## Update the screenshots

```sh
./run.sh --update-snapshots
```

Look at every changed PNG before committing it. A change that moves pixels on
purpose commits its new baselines in the same commit as the change.

## Telemetry

`run.sh` and the CI job set `E2E_TELEMETRY_DISABLED=1`, `DO_NOT_TRACK=1`,
and `E2E_UPDATE_CHECK_DISABLED=1`. On your own machine, `npx e2e telemetry
disable` turns it off for every run.
