# ADR 0001: Frontend stack

- Status: proposed (waiting for Zhafron's decision on ZHA-4133)
- Date: 2026-10-28
- Base: `origin/main` 6c70754
- Scope: research and decision only. Nothing in `web/`, `crates/` or `src-tauri/` changes until this ADR is accepted.

## Context

`web/` is hand-written HTML, CSS and classic scripts with no build step:

| File | Raw bytes | gzip -9 bytes |
|---|---:|---:|
| `index.html` | 27,432 | 6,366 |
| `app.js` | 93,267 | 23,542 |
| 9 other scripts (`playback`, `device-settings`, `fisheye`, `grid`, `alarms`, `device-admin`, `talk`, `account`, `recordings`) | 113,102 | 32,813 |
| `style.css` + `tokens.css` | 26,837 | 5,745 |
| **Text total** | **261,638** | **68,466** (per file) |
| Fonts (5 woff2) | 100,620 | already compressed |

That is 6,833 lines on `origin/main` (the issue text counted 4 files on an older checkout). The scripts share state through globals (`window.ProviewGrid`, `window.__PROVIEW_DESKTOP__`) and load in a fixed order from `index.html`.

Rocket already embeds every file in the binary: one route per file with `include_str!`/`include_bytes!` (`crates/server/src/routes.rs:117-190`, `routes/alarms.rs:42`, `routes/playback.rs:47`, `routes/settings_write.rs:429`, `desktop.rs:9`). Rocket serves them uncompressed. The desktop shell asks Rocket for the page over loopback; `desktop.rs` injects a bootstrap `<script>` before `</head>` of the embedded `index.html`.

The 26 UI tests in `tests/web/*.test.mjs` drive the real server in headless Chrome over CDP. CI runs them in the `check` job.

## How the numbers were produced

Every number below comes from a run on this machine on 2026-10-28: WSL2 on Fedora 44, Node 24.18.0, npm 11.16.0, rustc 1.99.1. Machine load during the runs was between 5 and 14 (`uptime`), so treat small differences as noise. Everything needed to reproduce them is under `tools/frontend-bench/` and `tools/webview-probe/`.

Each option is the same small app: a device rail that fetches `GET /api/devices` into a typed `Device[]`, an online counter, a selected-device state with `aria-pressed`, a `<video>` element, and one CSS file using `color-mix` and `:focus-visible`. It measures framework and tooling overhead, not the cost of porting this app.

```sh
cd tools/frontend-bench
./bench.sh
PATH=<dir with trunk 0.22.1>:$PATH SLOT=~/re/proview-cctv/build-slot.sh ./bench-wasm.sh
./first-frame.sh
(cd embed && SLOT=~/re/proview-cctv/build-slot.sh ./measure.sh)
```

- `bench.sh` runs `npm ci` (empty npm cache, `nice -n 10`) and then three `npm run build` runs per JavaScript option. It reports gzip -9 bytes of every emitted `.js`, `.css` and `.html` file.
- `bench-wasm.sh` runs `trunk build --release` (opt-level z, LTO, `wasm-opt -Oz`) through `build-slot.sh` (`CARGO_BUILD_JOBS=2`, nice 10). It does a cold build in an empty target directory, then a warm build after touching `main.rs`.
- `first-frame.mjs` serves a `dist/`, answers `/api/devices` with 8 devices and launches a fresh browser profile. It reports `first-contentful-paint` and `ready`: the first animation frame after all 9 rail buttons are in the DOM. Each figure is the median of 5 launches.
- `tools/webview-probe/run.sh <engine>` loads `probe.html` and records feature detection results.

## Option matrix

First-frame columns: Chrome is headless Chrome 156. WebKitGTK is MiniBrowser on WebKitGTK 2.54.1 (Fedora 44). Format is median FCP / ready, in ms.

| Option | Versions | JS gz | Total gz (JS+CSS+HTML) | `npm ci` s | Build s (3 runs) | Chrome FCP/ready | WebKitGTK FCP/ready | Node at build | Packages installed | Licence (runtime) | Weekly downloads (npm, 2026-10-20..26) |
|---|---|---:|---:|---:|---|---|---|---|---:|---|---:|
| **Vanilla + Vite + TS** | vite 8.3.5, ts 7.0.2 | 735 | 1,321 | 3.0 | 0.9 / 0.4 / 0.5 | 132 / 132 | 19 / 49 | yes (`^20.19 \|\| >=22.12`) | 18 | MIT (vite) | vite 171.6 M |
| **Vite + Vue 3 + TS** | vue 3.5.44, vue-tsc 3.3.13, ts 6.0.3 | 24,673 | 25,303 | 3.2 | 1.9 / 1.6 / 2.5 (includes `vue-tsc`) | 128 / 128 | 37 / 58 | yes | 49 | MIT | vue 15.6 M |
| **Vite + React + TS** | react 19.3.1 | 67,937 | 68,568 | 1.6 | 1.4 / 0.6 / 0.7 (includes `tsc -b`) | 156 / 156 | 86 / 133 | yes | 29 | MIT | react 165.7 M |
| **Vite + Solid + TS** | solid-js 1.9.17 | 5,477 | 6,108 | 2.2 | 0.8 / 0.8 / 0.7 | 156 / 157 | 30 / 65 | yes | 79 | MIT | solid-js 7.0 M |
| **Vite + Svelte 5 + TS** | svelte 5.57 | 13,260 | 13,894 | 1.9 | 1.3 / 0.9 / 0.6 (no type check in the template build) | 144 / 137 | 34 / 70 | yes | 50 | MIT | svelte 5.9 M |
| **SvelteKit (SPA, adapter-static)** | @sveltejs/kit 3.1.0 | 34,359 (11 files) | 35,275 | 6.3 | 4.6 / 2.7 / 4.5 | 272 / 272 | 70 / 86 | yes | 56 | MIT | 3.0 M |
| **SolidStart (`ssr: false`)** | @solidjs/start 2.1.0, nitro 3 beta | 5,868 | 6,232, **no `index.html`** | 3.2 | 3.8 / 3.4 / 4.4 | not measured | not measured | yes, **and at run time** (see below) | 181 | MIT | 0.11 M |
| **Astro (islands, Vue island)** | astro 7.4.2 | 29,438 (3 files) | 32,068 | 7.6 | 2.8 / 2.7 / 1.9 | 144 / 165 | 26 / 78 | yes (`>=22.23.4`) | 290 | MIT | 7.3 M |
| **Leptos (CSR, trunk)** | leptos 0.8.22 | 6,440 JS + 60,625 wasm | 68,033 | n/a | cold 73.3, warm 4.2 | 152 / 145 | 57 / 81 | no; trunk downloads wasm-bindgen and wasm-opt | 180 crates | MIT | crates.io 1.51 M / 90 d |
| **Yew (CSR, trunk)** | yew 0.23.0 | 6,339 JS + 69,939 wasm | 77,243 | n/a | cold 28.9, warm 2.6 | 168 / 168 | not measured | same as Leptos | 94 crates | MIT OR Apache-2.0 | 0.44 M / 90 d |
| **Dioxus (web, trunk)** | dioxus 0.7.10 | 18,138 JS + 176,571 wasm | 196,183 | n/a | cold 38.0, warm 4.4 | 168 / 180 | not measured | same as Leptos | 343 crates | MIT OR Apache-2.0 | 1.28 M / 90 d |
| *Today's `web/` (reference)* | - | 56,355 (10 files) | 68,466 | - | - | first paint 116, no FCP | not measured | no | 0 | MIT | - |

Notes on the matrix:

1. **What "ready" measures.** The Chrome numbers vary by up to ±100 ms between launches (React: 116-876 ms), so Chrome does not separate the options. WebKitGTK is the slowest engine this app ships on, and there the ranking is stable: vanilla < Vue ≈ Solid ≈ Svelte ≈ Astro < Leptos ≈ SvelteKit < React.
2. **SolidStart.** With `ssr: false` it still builds a Nitro server, and the HTML shell comes from that server at run time. `.output/public` has no `index.html`. With `nitro({ preset: "static" })` the build fails: "rolldownOptions.input should not be an html file when building for SSR". Rocket cannot serve that output without Node running next to it, so its first-frame rows are not measured.
3. **Yew and Dioxus on WebKitGTK.** I stopped this run when the MiniBrowser windows from `xvfb-run` showed up on the owner's WSLg desktop: GTK picked the WSLg Wayland socket and ignored Xvfb. The scripts now set `GDK_BACKEND=x11`. These rows were not re-run on the desktop. Re-running them needs `ENGINES=webkitgtk ./first-frame.sh`, or the Docker image in `tools/webview-probe/Dockerfile`.
4. **Today's `web/`.** Without the Rocket backend the real UI stays on its text-free skeleton, so `first-contentful-paint` never fires. First paint (116 ms in Chrome) is the only comparable figure. A real before/after has to be measured against the running server in migration step 1.
5. **WASM build time.** WASM build time is for `CARGO_BUILD_JOBS=2` at nice 10, without a shared cache. It does not include installing the `wasm32-unknown-unknown` target or the trunk, wasm-bindgen and wasm-opt downloads. The first Leptos and Yew attempts failed because the wasm-bindgen download from GitHub timed out. That means a network call at build time, which a reproducible build has to pin or vendor.

## Webview support

Feature probe: `tools/webview-probe/probe.html`. The commands are `./run.sh webkitgtk`, `./run.sh ubuntu-22.04`, `./run.sh ubuntu-24.04` and `./run.sh chrome`. The Ubuntu rows run in Docker containers built from `Dockerfile` (python3-gi + `gir1.2-webkit2-4.1` + Xvfb, no GStreamer libav).

| Engine | WebCodecs `VideoDecoder` | `isConfigSupported` avc1 / hvc1 | MSE avc1 / hvc1 | `ManagedMediaSource` | `color-mix()` | Popover API | `:focus-visible` selector | `:focus-visible` after `el.focus()` | CSS nesting | WASM |
|---|---|---|---|---|---|---|---|---|---|---|
| WebKitGTK 2.54.1, Fedora 44 (this box) | yes | no / no | no / no | yes | yes | yes | yes | **yes** | yes | yes |
| WebKitGTK 2.52.6, Ubuntu 24.04 | yes | no / no | no / no | yes | yes | yes | yes | **no** | yes | yes |
| WebKitGTK 2.50.4, Ubuntu 22.04 (the CI `desktop-smoke` runner) | yes | no / no | no / no | yes | yes | yes | yes | **no** | yes | yes |
| Chrome 156 headless | yes | yes / no | yes / no | no | yes | yes | yes | yes | yes | yes |
| WebView2 runtime 155.0.4283.62 (installed on the Windows host) | not measured | | | | | | | | | |
| WKWebView (macOS) | not measured: no macOS host | | | | | | | | | |

What this means:

- No framework changes any of these rows. WebCodecs, MSE, Popover and `color-mix` are platform APIs that every option reaches the same way, through the DOM. The negative avc1 rows on WebKitGTK come from GStreamer plugins that are not installed (see `docs/webview-matrix.md`), not from the webview.
- `:focus-visible` behaves differently across WebKitGTK versions. On 2.50 and 2.52, an element focused from script does not match `:focus-visible`; on 2.54 it does. The design's focus ring (`DESIGN.md`, "Focus and targets") and the menus' roving focus move focus from script. On the WebKitGTK that CI builds against, a ring that 2.54 shows can therefore be missing. This applies to the current code just as much as to any rewrite. The parity proof for each migration step has to include this engine.
- WebView2 is Chromium 155, so `[INFERENCE]` it should match the Chrome row, but nothing ran there. WebView2 and WKWebView stay as release-runner checks, as `docs/webview-matrix.md` already says.

## Serving the assets from the binary

I built a standalone Rocket 0.5.2 crate (`tools/frontend-bench/embed`, release, thin LTO) that serves the 19 real files from `web/`. For each embedding mode it records binary size and the time from process start to the first `200` on `/`, as the median of 10 launches.

| Mode | Binary bytes | Delta vs no assets | Start → first 200 (ms, median) | `/` bytes on the wire |
|---|---:|---:|---:|---:|
| No assets | 4,203,320 | - | 7 | - |
| `include_bytes!` table (today's approach) | 4,572,840 | +369,520 | 7 | 27,432 |
| `rust-embed` 8 on the directory | 4,574,776 | +371,456 | 7 | 27,432 |
| `rust-embed` on gzip -9 precompressed files, `Content-Encoding: gzip` | 4,379,560 | +176,240 | 8 | 6,366 |

The embedding mode makes no measurable difference to start time; every mode lands at 7-8 ms. `rust-embed` and per-file routes cost the same bytes. The difference that matters is in maintenance: hashed Vite file names (`index-BF-GHj1k.js`) cannot be listed by hand as Rocket routes, so a bundler needs a directory embed. Precompressed gzip halves both the binary delta and the bytes sent. It needs an `Accept-Encoding` check and a fallback for clients that do not send it, so it is a separate later step.

## Type safety from Rust

I did a trial on a scratch copy of `origin/main`. I added `ts-rs = "12"` to `proview-core` and `#[derive(ts_rs::TS)] #[ts(export)]` to `Device` and `CloudPath`, then ran:

```sh
OPENSSL_NO_VENDOR=1 TS_RS_EXPORT_DIR=bindings ~/re/proview-cctv/build-slot.sh cargo test -p proview-core --lib export_bindings
```

It passed 2 tests in 95 s (a cold target directory) and wrote:

```ts
export type Device = { id: string, name: string, model: string, online: boolean, channels: number, lan_host: string | null, online_checked_at?: bigint | null, online_status?: string | null, online_detail?: string | null, };
export type CloudPath = { online: boolean, access: string | null, agent: string | null, relayAccepted: boolean, relayError: string | null, destIp: string | null, mtu: string | null, detail: string, };
```

What the trial showed:

- ts-rs follows serde attributes: `rename_all = "camelCase"` and `skip_serializing_if` both come through. It also shows that the API mixes snake_case (`Device`) and camelCase (`CloudPath`). One generated source makes that visible instead of hiding it.
- `u64` maps to `bigint`, but `serde_json` sends a JSON number and `JSON.parse` returns `number`. Every `u64`/`i64` field needs `#[ts(type = "number")]`, or the generated type is wrong.
- 142 `Serialize` derives exist in `crates/`. Only the types that `/api/*` returns need the derive, and the generated files must be checked in CI (`git diff --exit-code` after the export test). Both apply to every frontend option. With Leptos, Yew or Dioxus the structs are shared directly, which is their one structural advantage.

## Decision

Chosen: **Vite + TypeScript, introduced first with the existing vanilla code; Vue 3 added view by view on top of it.** Svelte 5 is the close runner-up (see Rejected).

Reasons:

1. **Vite + TypeScript is the base for every viable option.** On its own it adds 735 B gzip and builds in under a second. It gives what `web/` lacks today: modules instead of globals, type checks, one place for the generated `/api/*` types, and hashed assets. It is also the migration path, because the existing scripts can move behind it unchanged.
2. **Vue 3 is what was asked for, and the measurements allow it.** Its cost over vanilla is 23.9 KB gzip, which on loopback means parse time and nothing else. On WebKitGTK that is 58 ms against 49 ms to ready, the closest of any framework. Its single-file component template is plain HTML, which keeps the 287 lines of approved `index.html` markup and the `redesign-v1` structure close to how they look now. `vue-tsc` type-checks templates against the generated types. Of the template frameworks it has the largest ecosystem (15.6 M weekly).
3. **The state bugs are where a framework pays off.** ZHA-4131 (Settings edits lost when the boot finishes) is a state-ownership bug in hand-written DOM code. Settings, alarms, device admin and playback lists are form and list views. The live frame path is not: it stays a plain TS module that Vue only mounts and never makes reactive.
4. **Tooling stays small.** There is one `web/package.json` with a committed `package-lock.json`, and npm comes with Node, so no extra tool is needed. One JavaScript package next to a Cargo workspace does not justify pnpm workspaces, Turborepo or Nx. The "monorepo" stays what it is: the Cargo workspace plus `web/`.
5. **Embedding** uses `rust-embed` on `web/dist`. It replaces the per-file routes and costs the same bytes and start time as today (measured above). Precompressed gzip is an optional later step that saves 195 KB of binary.

## Rejected

| Option | Why not |
|---|---|
| Vite + React | It has the largest runtime (67.9 KB gzip, 2.8× Vue) and the slowest WebKitGTK ready time (133 ms against 58). Its JSX moves away from the approved HTML markup. |
| SvelteKit | Its router, load functions and adapters serve a server-rendered site that this app does not have. It emits 11 JS files, has the slowest Chrome ready time (272 ms) and is the slowest of the Vite options to build (2.7-4.6 s). |
| SolidStart | `ssr: false` still needs a Nitro server for the HTML shell, the static preset fails to build, and the server output needs Node at run time. That breaks "one binary, Rocket serves everything". It has 0.11 M weekly downloads. |
| Astro | It is built for content sites and multi-page output; this app is one window. It installs 290 packages and needs Node ≥ 22.23.4. An island still needs Vue or another framework, so it adds a layer without removing one. |
| Leptos / Yew / Dioxus | They ship 68-196 KB gzip of wasm+JS, against 25 KB for Vue. Cold builds take 29-73 s on two jobs, before installing the wasm target and downloading trunk, wasm-bindgen and wasm-opt. That breaks the 60 s budget for the `check` job and adds a build-time network call. The video path (WebSocket, MSE, WebCodecs, canvas) would go through `web-sys` bindings for no gain. Sharing Rust structs is their one advantage, and ts-rs gives the other options most of it. |
| Vite + Solid | It is the smallest framework runtime (5.5 KB) and fast on WebKitGTK (65 ms). Its JSX and the rule that props must not be destructured make it easy to get reactivity subtly wrong, and its ecosystem is about half of Vue's. Within noise of Vue on speed, so the deciding factors are familiarity and templates. |
| Vite + Svelte 5 | **Runner-up.** It is 13.3 KB gzip with HTML-like templates and type checking (`svelte-check`), and 70 ms ready on WebKitGTK. If Zhafron prefers the smaller runtime, swap Vue for Svelte in steps 4-6 of the plan; steps 1-3 stay the same. |
| Stay vanilla without a build | The current state. There are no types, the API types are hand-written once in Rust and implicitly again in JS, the scripts share globals, and every new file needs a hand-written Rocket route. |

## Migration plan (each step is one PR; `main` stays shippable)

Every step must pass all of these before merge:

- **Tests.** The existing `tests/web/*.test.mjs` (26 tests) are green in CI, and the count never goes down.
- **Screenshots.** Screenshots of the touched views on Chromium and on WebKitGTK 2.50 (the Docker image under `tools/webview-probe`, never on the desktop) are compared with the previous `main` and with `docs/design/redesign-v1/`.
- **Size and speed.** JS gzip and first frame are measured against the running Rocket server and recorded in `docs/performance.md`, with before and after numbers.
- **Build time.** The `check` job grows by at most 60 s.

| Step | Change | Boundary | Proof that nothing regressed |
|---|---|---|---|
| 1 | Add `web/package.json` (vite, typescript, lockfile). Vite builds `index.html`, `tokens.css` and `style.css`; the scripts move unchanged into `web/public/` and are copied as-is. Rocket serves `web/dist` through `rust-embed`, replacing the per-file routes and keeping `/api/*`, `/ws/*` and `desktop.rs` injection. CI runs `npm ci && npm run build` before `cargo`. `release.yml` builds `web/dist` once and gives it to every cross target. | No JS behaviour change | `sha256` of every served script is equal before and after. All 26 UI tests pass. Measured binary delta and start time. `desktop-smoke` is green on 3 OS. |
| 2 | Turn the scripts into ES modules imported from `src/main.ts` in the current load order. Turn on `checkJs` in `tsconfig.json` (no source comments), and fix only type errors. | Globals stay as exports | UI tests pass. Bundle gzip stays at or below today's 56 KB of JS. |
| 3 | Add ts-rs to the response types of `/api/*` (with `#[ts(type = "number")]` on integer fields) and export into `web/src/api/generated/`. Add a CI check that fails on drift. Replace hand-written response handling with the generated types. | Rust structs only gain a derive | `cargo test` passes and the drift check is green. A changed field breaks `tsc`, proven once with a deliberate mutation. |
| 4 | Add Vue 3. Port **Settings** first (form state; fixes the class of ZHA-4131), mounted into its existing container. | One view per PR | Settings tests plus a new test for edits kept across the boot refresh. Screenshot diff. |
| 5 | Port account/sign-in, alarms, device admin, playback list and recordings, one per PR. | Same | The tests for each view, screenshot diff, focus-ring check on WebKitGTK 2.50. |
| 6 | Port the live shell (camera picker, menus, PTZ, lens, grid). The frame path (WebSocket, MSE/WebCodecs, fisheye canvas, talk) stays plain TS modules behind a component, with no reactive state per frame. | Last, after 4-5 are stable | `ui.test.mjs`, `grid.test.mjs`, `snapshot.test.mjs`. Cold and warm first decoded frame measured as in `docs/performance.md` must not get worse. |
| 7 | Remove `web/public/*.js`, the globals and the leftover routes. | Only deletions | Every test green, and `index.html` has no classic `<script>`. |

Rollback: each step is one revertable PR. Steps 1-3 change no user-visible behaviour.

## Risks

1. **WebKitGTK versions differ** (2.50 on the CI runner, 2.52 on Ubuntu 24.04, 2.54 here). The `:focus-visible` difference above is real today. The probe image has to run on every UI PR, not only on releases.
2. **WebView2 and WKWebView have not been exercised**, here or on `main`. The plan does not make that worse, but it does not fix it either.
3. **UI/UX loss against `redesign-v1`.** Porting markup into components invites "improvements". The rule is that `tokens.css` and `style.css` carry over byte for byte in step 1, `tools/contrast.py` keeps running, and any visual change needs its own approved design change, not a port PR.
4. **Node becomes a build dependency** for `check`, `release` and `desktop-smoke`, on 3 OS and in `Cross` containers. Vite 8 ships Rolldown as native optional packages per platform, so the lockfile must resolve on Linux, Windows and macOS. That is `[INFERENCE]`, not tested here, and the first step has to prove it in CI on all three.
5. **Two runtimes during migration.** While steps 4-6 run, Vue and the legacy modules share the page. Shared state must go through one store module, or the ZHA-4131 class of bug comes back.
6. **Frame-path performance.** Vue reactivity must not wrap the video, canvas or WebSocket objects. Step 6 is the one most likely to regress, so it comes last and has the measured gate.
7. **TypeScript 7 and `vue-tsc`.** The Vue template still pins TypeScript 6.0.3 because `vue-tsc` does not accept 7 yet; the other templates use 7.0.2. A TS upgrade waits for `vue-tsc`.
8. **The numbers are from one loaded machine.** Treat the Chrome first-frame differences under about 100 ms as noise.
