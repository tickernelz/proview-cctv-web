# ADR 0001: Frontend stack

- Status: accepted (Zhafron, 2026-10-28 11:58 UTC): Vite + Vue 3, full migration of `web/`
- Date: 2026-10-28
- Base: `origin/main` 6c70754
- Scope: research and decision only. The migration itself is separate work, which starts after PR #63 (live cloud) is released.
- Evidence: tag `adr-0001-evidence` (commit fb87d0c) holds `tools/frontend-bench/`, `tools/webview-probe/` and every result file this ADR cites; run the commands below from a checkout of that tag.
- Outcome (2026-10-30, v0.4.0): the page now lives in `apps/web` (`apps/web/package.json`: vue, reka-ui, tailwindcss, vite, vue-tsc) and is embedded from `apps/web/dist` through `rust-embed`; `web/` and its classic scripts are gone. UI tests are `tests/web/*.test.mjs` plus the Playwright suite in `e2e/`. Step 3 (ts-rs types) has not landed, and `tools/frontend-bench/` and `tools/webview-probe/` exist only at the tag above.

## Context

`web/` is hand-written HTML, CSS and classic scripts with no build step:

| File | Raw bytes | gzip -9 bytes |
|---|---:|---:|
| `index.html` | 27,432 | 6,366 |
| `app.js` | 93,267 | 23,542 |
| 9 other scripts (`playback`, `device-settings`, `fisheye`, `grid`, `alarms`, `device-admin`, `talk`, `account`, `recordings`) | 114,102 | 32,813 |
| `style.css` + `tokens.css` | 26,837 | 5,745 |
| **Text total** | **261,638** | **68,466** (sum of per-file gzip) |
| Fonts (5 woff2) | 100,620 | already compressed |

That is 6,833 lines on `origin/main` 6c70754, `index.html` 296 of them (the issue text counted 4 files and 287 lines on an older checkout). gzip figures in this ADR are `gzip -9c <file> | wc -c`, so each one includes the file name in the gzip header (a few dozen bytes per file). The scripts share state through globals (`window.ProviewGrid`, `window.__PROVIEW_DESKTOP__`) and load in a fixed order from `index.html`.

Rocket already embeds every file in the binary: one route per file with `include_str!`/`include_bytes!` (`crates/server/src/routes.rs:117-190`, `routes/alarms.rs:42`, `routes/playback.rs:47`, `routes/settings_write.rs:429`, `desktop.rs:9`). Rocket serves them uncompressed. The desktop shell asks Rocket for the page over loopback; `desktop.rs` injects a bootstrap `<script>` before `</head>` of the embedded `index.html`.

The 26 UI tests in `tests/web/*.test.mjs` drive the real server in headless Chrome over CDP. CI runs them in the `check` job.

## How the numbers were produced

Every number below comes from a run on this machine on 2026-10-28: WSL2 on Fedora 44, Node 24.18.0, npm 11.16.0, rustc 1.99.1. Machine load during the runs was between 5 and 16 (`uptime`), so treat small differences as noise. Everything needed to reproduce them is under `tools/frontend-bench/` and `tools/webview-probe/` at tag `adr-0001-evidence`.

Each option is the same small app: a device rail that fetches `GET /api/devices` into a typed `Device[]`, an online counter, a selected-device state with `aria-pressed`, a `<video>` element, and one CSS file using `color-mix` and `:focus-visible`. It measures framework and tooling overhead, not the cost of porting this app.

```sh
cd tools/frontend-bench
./bench.sh
PATH=<dir with trunk 0.22.1>:$PATH SLOT=~/re/proview-cctv/build-slot.sh ./bench-wasm.sh
ENGINES="chrome ubuntu-22.04 ubuntu-24.04 fedora-44" ./first-frame.sh
(cd embed && SLOT=~/re/proview-cctv/build-slot.sh ./measure.sh)
```

- `bench.sh` runs `npm ci` (empty npm cache, `nice -n 10`) and then three `npm run build` runs per JavaScript option. It reports gzip -9 bytes of every emitted `.js`, `.css` and `.html` file.
- `bench-wasm.sh` runs `trunk build --release` (opt-level z, LTO, `wasm-opt -Oz`) through `build-slot.sh` (`CARGO_BUILD_JOBS=2`, nice 10). It does a cold build in an empty target directory, then a warm build after touching `main.rs`.
- `first-frame.sh` (`ENGINES="chrome ubuntu-22.04 ubuntu-24.04 fedora-44"`; default `chrome` only) runs `first-frame.mjs` per option. It serves a `dist/`, answers `/api/devices` with 8 devices and loads the page in a fresh browser: headless Chrome on the host, or a WebKitGTK WebView inside an isolated container (see Webview support). It reports `first-contentful-paint` and `ready`: the first animation frame after all 9 rail buttons are in the DOM, 5 launches per option, every launch kept.
- `tools/webview-probe/run.sh <engine>` loads `probe.html` and records feature detection results.
- Packages installed: `npm ls --all --parseable --include=dev | wc -l` in each option, or `grep -c '^name = ' Cargo.lock` for the WASM options. Weekly downloads: `https://api.npmjs.org/downloads/point/last-week/<package>`; crates: `recent_downloads` (90 days) from `https://crates.io/api/v1/crates/<crate>`.
- Reproducible build: `bench.sh` was run again on a `git archive` copy in another directory, and every file of each output directory was compared by `sha256sum`.
- All scripts write to `*.tmp` by default, so re-running them does not overwrite the committed results.

## Option matrix

| Option | Versions | JS gz | Total gz (JS+CSS+HTML) | `npm ci` s | Build s (3 runs) | Byte-reproducible build | Node at build | Packages installed | Licence (runtime) | Weekly downloads (npm, 2026-10-20..26) |
|---|---|---:|---:|---:|---|---|---|---:|---|---:|
| **Vanilla + Vite + TS** | vite 8.3.5, ts 7.0.2 | 735 | 1,321 | 3.0 | 0.9 / 0.4 / 0.5 | yes | yes (`^20.19 \|\| >=22.12`) | 18 | MIT (vite) | vite 171.6 M |
| **Vite + Vue 3 + TS** | vue 3.5.44, vue-tsc 3.3.13, ts 6.0.3 | 24,673 | 25,303 | 3.2 | 1.9 / 1.6 / 2.5 (includes `vue-tsc`) | yes | yes | 49 | MIT | vue 15.6 M |
| **Vite + React + TS** | react 19.3.1 | 67,937 | 68,568 | 1.6 | 1.4 / 0.6 / 0.7 (includes `tsc -b`) | yes | yes | 29 | MIT | react 165.7 M |
| **Vite + Solid + TS** | solid-js 1.9.17 | 5,477 | 6,108 | 2.2 | 0.8 / 0.8 / 0.7 | yes | yes | 79 | MIT | solid-js 7.0 M |
| **Vite + Svelte 5 + TS** | svelte 5.57 | 13,260 | 13,894 | 1.9 | 1.3 / 0.9 / 0.6 (no type check in the template build) | yes | yes | 50 | MIT | svelte 5.9 M |
| **SvelteKit (SPA, adapter-static)** | @sveltejs/kit 3.1.0 | 34,359 (11 files) | 35,275 | 6.3 | 4.6 / 2.7 / 4.5 | **no**: 2 files differ (`_app/version.json` timestamp, `__sveltekit_<hash>`) | yes | 56 | MIT | 3.0 M |
| **SolidStart (`ssr: false`)** | @solidjs/start 2.1.0, nitro 3 beta | 5,868 | 6,232, **no `index.html`** | 3.2 | 3.8 / 3.4 / 4.4 | yes | yes, **and at run time** (see below) | 181 | MIT | 0.11 M |
| **Astro (islands, Vue island)** | astro 7.4.2 | 29,438 (3 files) | 32,068 | 7.6 | 2.8 / 2.7 / 1.9 | **no**: 1 file differs (random `astro-island` uid) | yes (`>=22.23.4`) | 290 | MIT | 7.3 M |
| **Leptos (CSR, trunk)** | leptos 0.8.22 | 6,440 JS + 60,625 wasm | 68,033 | n/a | cold 73.3, warm 4.2 | not measured | no; trunk downloads wasm-bindgen and wasm-opt | 180 crates | MIT | crates.io 1.51 M / 90 d |
| **Yew (CSR, trunk)** | yew 0.23.0 | 6,339 JS + 69,939 wasm | 77,243 | n/a | cold 28.9, warm 2.6 | not measured | same as Leptos | 94 crates | MIT OR Apache-2.0 | 0.44 M / 90 d |
| **Dioxus (web, trunk)** | dioxus 0.7.10 | 18,138 JS + 176,571 wasm | 196,183 | n/a | cold 38.0, warm 4.4 | not measured | same as Leptos | 343 crates | MIT OR Apache-2.0 | 1.28 M / 90 d |
| *Today's `web/` (reference)* | - | 56,355 (10 files) | 68,466 | - | - | - (no build) | no | 0 | MIT | - |

### First frame

`tools/frontend-bench/results-first-frame.jsonl` holds every launch. Columns: median `ready` in ms over 5 launches, with the range of the 5 in brackets. Chrome is headless Chrome 156 on the host; the WebKitGTK columns run a plain WebView in the Docker images above. Load average during the run was 7-16.

| Option | Chrome 156 | WebKitGTK 2.50.4 | WebKitGTK 2.52.6 | WebKitGTK 2.54.2 |
|---|---|---|---|---|
| Vanilla + Vite + TS | 152 [123-204] | 82 [74-118] | 380 [96-408] | 406 [354-506] |
| Vue 3 | 160 [126-262] | 79 [66-106] | 175 [120-210] | 58 [55-499] |
| React | 172 [79-299] | 74 [69-117] | 124 [100-148] | 86 [66-155] |
| Solid | 134 [119-163] | 79 [72-93] | 122 [93-388] | 151 [131-483] |
| Svelte 5 | 168 [136-191] | 84 [82-366] | 96 [85-383] | 106 [60-591] |
| SvelteKit | 206 [190-272] | 93 [76-103] | 110 [99-146] | 123 [112-226] |
| Astro | 168 [162-260] | 74 [61-106] | 353 [313-395] | 501 [406-547] |
| Leptos | 148 [136-218] | 74 [65-96] | 105 [91-160] | 88 [72-104] |
| Yew | 359 [174-1454] | 113 [79-124] | 111 [95-159] | 174 [111-278] |
| Dioxus | 201 [155-235] | 109 [96-287] | 139 [129-286] | 182 [106-1108] |
| SolidStart | not measured | not measured | not measured | not measured |
| Today's `web/` | not comparable | not comparable | not comparable | not comparable |

Notes on the matrix:

1. **First frame does not separate the options by size.** On WebKitGTK each launch falls into one of two modes. When `first-contentful-paint` fires more than 20 ms before `ready`, `ready` lands at 287-1108 ms (29 of 150 launches: 2 on 2.50, 12 on 2.52, 15 on 2.54). When FCP and `ready` coincide, `ready` lands at 55-286 ms (121 of 150). So the medians in the table mostly report which mode a build tended to hit: on 2.54.2 vanilla [354-506] and Astro [406-547] do not overlap React [66-155] or Leptos [72-104], and on 2.52.6 Astro [313-395] does not overlap React [100-148]. `[INFERENCE]` The slow mode is a first paint of the empty or partial page followed by a delayed second paint once the device list renders. Which mode a build hits does not follow its size: the 1,321-byte vanilla build is in the slow mode on 2.54, the 68 KB React build in the fast one. On loopback, framework size does not show up in the time to first frame at this app size; the decision below does not rest on these numbers. An earlier round of this ADR ranked the options on a single WebKitGTK run of the host `MiniBrowser`; that ranking is withdrawn because it did not repeat.
2. **SolidStart.** With `ssr: false` it still builds a Nitro server, and the HTML shell comes from that server at run time. `.output/public` has no `index.html`. With `nitro({ preset: "static" })` the build fails: "rolldownOptions.input should not be an html file when building for SSR". Rocket cannot serve that output without Node running next to it, so its first-frame rows are not measured.
3. **Today's `web/`.** Without the Rocket backend the real UI stays on its text-free skeleton, so `first-contentful-paint` never fires and `ready` does not apply. A real before/after has to be measured against the running server in migration step 1.
4. **WASM build time.** WASM build time is for `CARGO_BUILD_JOBS=2` at nice 10, without a shared cache. It does not include installing the `wasm32-unknown-unknown` target or the trunk, wasm-bindgen and wasm-opt downloads. The first Leptos and Yew attempts failed because the wasm-bindgen download from GitHub timed out. That means a network call at build time, which a reproducible build has to pin or vendor.
5. **Reproducible build** was not checked for the WASM options.

## Webview support

Feature probe: `tools/webview-probe/probe.html`, run with `tools/webview-probe/run.sh <engine>`. Build the images first with `tools/webview-probe/build-images.sh` (`Dockerfile.ubuntu`: python3-gi + `gir1.2-webkit2-4.1` + Xvfb; `Dockerfile.fedora`: python3-gobject + gobject-introspection + `webkit2gtk4.1` + Xvfb; no GStreamer libav in either). Each WebKitGTK engine runs as a plain `WebKit2.WebView` in a GTK window (`gtk_probe.py`) inside its container with `--network none`: the page server (`serve.py`) and Xvfb run in the same container, so nothing can reach the host display. `chrome` runs headless Chrome on the host.

| Engine | WebCodecs `VideoDecoder` | `isConfigSupported` avc1 / hvc1 | MSE avc1 / hvc1 | `ManagedMediaSource` | `color-mix()` | Popover API | `:focus-visible` selector | `:focus-visible` after `el.focus()`, no key pressed | CSS nesting | WASM |
|---|---|---|---|---|---|---|---|---|---|---|
| WebKitGTK 2.54.2, Fedora 44 (`fedora-44`) | yes | no / no | no / no | yes | yes | yes | yes | no | yes | yes |
| WebKitGTK 2.52.6, Ubuntu 24.04 (`ubuntu-24.04`) | yes | no / no | no / no | yes | yes | yes | yes | no | yes | yes |
| WebKitGTK 2.50.4, Ubuntu 22.04 (`ubuntu-22.04`, the CI `desktop-smoke` runner) | yes | no / no | no / no | yes | yes | yes | yes | no | yes | yes |
| Chrome 156 headless | yes | yes / no | yes / no | no | yes | yes | yes | yes | yes | yes |
| WebView2 runtime 155.0.4283.62 (installed on the Windows host) | not measured | | | | | | | | | |
| WKWebView (macOS) | not measured: no macOS host | | | | | | | | | |

What this means:

- No framework changes any of these rows. WebCodecs, MSE, Popover and `color-mix` are platform APIs that every option reaches the same way, through the DOM. The negative avc1 rows on WebKitGTK come from GStreamer plugins that are not installed (see `docs/webview-matrix.md`), not from the webview.
- `:focus-visible` after a script `el.focus()` with no prior key press does not match on any of the three WebKitGTK versions, and does match in Chrome. An earlier round of this ADR reported "yes" for 2.54.1; that run used the host's `MiniBrowser` application, not a plain WebView, so `[INFERENCE]` the difference came from the host app rather than the version; both differ (host MiniBrowser 2.54.1 against a plain WebView on 2.54.2), so this run cannot separate them. WebKit decides `:focus-visible` from the last input modality, and the menus' roving focus runs inside key handlers, so `[INFERENCE]` the ring after a real arrow key press still shows on WebKitGTK; this probe did not press a key. Where focus moves without prior keyboard input (for example on view load), WebKitGTK and Chrome differ today, in the current code as much as in any rewrite.
- WebView2 is Chromium 155, so `[INFERENCE]` it should match the Chrome row, but nothing ran there. WebView2 and WKWebView stay as release-runner checks, as `docs/webview-matrix.md` already says.

## Serving the assets from the binary

I built a standalone Rocket 0.5.2 crate (`tools/frontend-bench/embed`, release, thin LTO) that serves the 19 real files from `web/`. For each embedding mode it records binary size and the time from process start to the first `200` on `/`, as the median of 10 launches.

| Mode | Binary bytes | Delta vs no assets | Start → first 200 (ms, median) | `/` bytes on the wire |
|---|---:|---:|---:|---:|
| No assets | 4,203,320 | - | 7 | - |
| `include_bytes!` table (today's approach) | 4,572,840 | +369,520 | 7 | 27,432 |
| `rust-embed` 8 on the directory | 4,574,776 | +371,456 | 7 | 27,432 |
| `rust-embed` on gzip -9 precompressed files, `Content-Encoding: gzip` | 4,379,560 | +176,240 | 8 | 6,366 |

The embedding mode makes no measurable difference to start time; every mode lands at 7-8 ms. `rust-embed` and per-file routes cost the same bytes. The difference that matters is in maintenance: hashed Vite file names (`index-BF-GHj1k.js`) cannot be listed by hand as Rocket routes, so a bundler needs a directory embed. Precompressed gzip cuts the binary delta to 0.47× (371,456 → 176,240 bytes) and the bytes sent for `/` to 0.23× (27,432 → 6,366). It needs an `Accept-Encoding` check and a fallback for clients that do not send it, so it is a separate later step.

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
- 129 `Serialize` derives exist in `crates/` at 6c70754 (`git grep -h -E "derive\([^)]*\bSerialize\b" 6c70754 -- crates/ | wc -l`), plus 1 in `src-tauri`. Only the types that `/api/*` returns need the derive, and the generated files must be checked in CI (`git diff --exit-code` after the export test). Both apply to every frontend option. With Leptos, Yew or Dioxus the structs are shared directly, which is their one structural advantage.

## Decision

Chosen: **Vite + TypeScript, introduced first with the existing vanilla code; Vue 3 added view by view on top of it.** Svelte 5 is the close runner-up (see Rejected).

Accepted with two conditions from Zhafron: the migration is complete, so every view ends in Vue 3 (step 7 of the plan is required, not optional), and the approved UI/UX of `redesign-v1` (direction B, "Window") is kept as the baseline. Improvements to it are allowed during the port; each one is named in its PR rather than slipped into a port step.

Reasons:

1. **Vite + TypeScript is the base for every viable option.** On its own it adds 735 B gzip and builds in under a second. It gives what `web/` lacks today: modules instead of globals, type checks, one place for the generated `/api/*` types, and hashed assets. It is also the migration path, because the existing scripts can move behind it unchanged.
2. **Vue 3 is what was asked for, and the measurements allow it.** Its cost over vanilla is 23.9 KB gzip; on loopback the first-frame runs show no difference between the options that this size would explain (note 1 under the matrix). Its build is byte-reproducible. Its single-file component template is plain HTML, which keeps the 296 lines of approved `index.html` markup and the `redesign-v1` structure close to how they look now. `vue-tsc` type-checks templates against the generated types. Of the template frameworks it has the largest ecosystem (15.6 M weekly).
3. **The state bugs are where a framework pays off.** Settings edits lost when the boot finishes (fixed in PR #61) is a state-ownership bug in hand-written DOM code. Settings, alarms, device admin and playback lists are form and list views. The live frame path is not: it stays a plain TS module that Vue only mounts and never makes reactive.
4. **Tooling stays small.** There is one `web/package.json` with a committed `package-lock.json`, and npm comes with Node, so no extra tool is needed. One JavaScript package next to a Cargo workspace does not justify pnpm workspaces, Turborepo or Nx. The "monorepo" stays what it is: the Cargo workspace plus `web/`.
5. **Embedding** uses `rust-embed` on `web/dist`. It replaces the per-file routes and costs the same bytes and start time as today (measured above). Precompressed gzip is an optional later step that saves 195 KB of binary.

## Rejected

| Option | Why not |
|---|---|
| Vite + React | It has the largest JS runtime of the JS options (67.9 KB gzip, 2.8× Vue). Its JSX moves away from the approved HTML markup. The first-frame runs do not show it slower (note 1), so speed is not the reason. |
| SvelteKit | Its router, load functions and adapters serve a server-rendered site that this app does not have. It emits 11 JS files, takes 2.7-4.6 s to build (only SolidStart is in the same range, 3.4-4.4 s), and its output is not byte-reproducible: two of its files differ between two builds of the same source. |
| SolidStart | `ssr: false` still needs a Nitro server for the HTML shell, the static preset fails to build, and the server output needs Node at run time. That breaks "one binary, Rocket serves everything". It has 0.11 M weekly downloads. |
| Astro | It is built for content sites and multi-page output; this app is one window. It installs 290 packages and needs Node ≥ 22.23.4. An island still needs Vue or another framework, so it adds a layer without removing one. |
| Leptos / Yew / Dioxus | They ship 68-196 KB gzip of wasm+JS, against 25 KB for Vue. trunk downloads wasm-bindgen and wasm-opt from GitHub at build time; that download timed out twice here (Leptos and Yew, first attempt), so a reproducible build has to pin and vendor those tools. The video path (WebSocket, MSE, WebCodecs, canvas) would go through `web-sys` bindings for no gain. Sharing Rust structs is their one advantage, and ts-rs gives the other options most of it. The CI budget is **not** a reason: the cold builds measured here (Leptos 73.3 s, Yew 28.9 s, Dioxus 38.0 s, 2 jobs at nice 10 under load 5-14) are not the CI number, `check` uses `Swatinem/rust-cache`, the warm builds were 2.6-4.4 s, and installing the wasm target and trunk on a runner was not measured. `[INFERENCE]` A real port of 6,833 lines costs more build time than these toy apps. |
| Vite + Solid | It is the smallest framework runtime (5.5 KB). Its JSX and the rule that props must not be destructured make it easy to get reactivity subtly wrong, and its ecosystem is about half of Vue's. Speed does not separate it from Vue (note 1), so the deciding factors are familiarity and templates. |
| Vite + Svelte 5 | **Runner-up.** It is 13.3 KB gzip with HTML-like templates and type checking (`svelte-check`), and its build is byte-reproducible. If Zhafron prefers the smaller runtime, swap Vue for Svelte in steps 4-6 of the plan; steps 1-3 stay the same. |
| Stay vanilla without a build | The current state. There are no types, the API types are hand-written once in Rust and implicitly again in JS, the scripts share globals, and every new file needs a hand-written Rocket route. |

## Migration plan (each step is one PR; `main` stays shippable)

Every step must pass all of these before merge:

- **Tests.** The existing `tests/web/*.test.mjs` (26 tests) are green in CI, and the count never goes down.
- **Screenshots.** Screenshots of the touched views on Chromium and on WebKitGTK 2.50 (the Docker image under `tools/webview-probe`, never on the desktop) are compared with the previous `main` and with `docs/design/redesign-v1/`.
- **Size and speed.** JS gzip and first frame are measured against the running Rocket server and recorded in `docs/performance.md`, with before and after numbers. On WebKitGTK, FCP and `ready` are recorded per launch together with the mode (FCP more than 20 ms before `ready`, or not), because the mode alone can flip a before/after: today's `web/` paints a skeleton first.
- **Build time.** The `check` job grows by at most 60 s.

| Step | Change | Boundary | Proof that nothing regressed |
|---|---|---|---|
| 1 | Add `web/package.json` (vite, typescript, lockfile). Vite builds `index.html`, `tokens.css` and `style.css`; the scripts move unchanged into `web/public/` and are copied as-is. Rocket serves `web/dist` through `rust-embed`, replacing the per-file routes and keeping `/api/*`, `/ws/*` and `desktop.rs` injection. CI runs `npm ci && npm run build` before `cargo`. `release.yml` builds `web/dist` once and gives it to every cross target. | No JS behaviour change | `sha256` of every served script is equal before and after, checked against a **release** binary. All 26 UI tests pass. Measured binary delta and start time. `desktop-smoke` is green on 3 OS. rust-embed 8 reads the files from disk in debug builds unless the `debug-embed` feature is on, and fails to compile when the folder is missing unless `#[allow_missing = true]`: the step turns on `debug-embed` so the debug binary the UI tests use serves the embedded bytes, and every `cargo` path (clippy, test, `desktop-smoke`, the local `build-slot.sh` loop) builds `web/dist` first. |
| 2 | Turn the scripts into ES modules imported from `src/main.ts` in the current load order. Turn on `checkJs` in `tsconfig.json` (no source comments), and fix only type errors. | Globals stay as exports | UI tests pass. Bundle gzip stays at or below today's 56 KB of JS. |
| 3 | Add ts-rs to the response types of `/api/*` (with `#[ts(type = "number")]` on integer fields) and export into `web/src/api/generated/`. Add a CI check that fails on drift. Replace hand-written response handling with the generated types. | Rust structs only gain a derive | `cargo test` passes and the drift check is green. A changed field breaks `tsc`, proven once with a deliberate mutation. |
| 4 | Add Vue 3. Port **Settings** first (form state; fixes the class of the PR #61 bug), mounted into its existing container. | One view per PR | Settings tests plus a new test for edits kept across the boot refresh. Screenshot diff. |
| 5 | Port account/sign-in, alarms, device admin, playback list and recordings, one per PR. | Same | The tests for each view, screenshot diff, focus-ring check on WebKitGTK 2.50. |
| 6 | Port the live shell (camera picker, menus, PTZ, lens, grid). The frame path (WebSocket, MSE/WebCodecs, fisheye canvas, talk) stays plain TS modules behind a component, with no reactive state per frame. | Last, after 4-5 are stable | `ui.test.mjs`, `grid.test.mjs`, `snapshot.test.mjs`. Cold and warm first decoded frame measured as in `docs/performance.md` must not get worse. |
| 7 | Remove `web/public/*.js`, the globals and the leftover routes. | Only deletions | Every test green, and `index.html` has no classic `<script>`. |

Rollback: each step is one revertable PR. Steps 1-3 change no user-visible behaviour.

## Risks

1. **WebKitGTK differs from Chrome**, and three versions ship (2.50 on the CI runner, 2.52 on Ubuntu 24.04, 2.54 on Fedora 44). The `:focus-visible` difference above is real today, and the first-frame spread on WebKitGTK is wider than on Chrome. The probe images have to run on every UI PR, not only on releases.
2. **WebView2 and WKWebView have not been exercised**, here or on `main`. The plan does not make that worse, but it does not fix it either.
3. **UI/UX loss against `redesign-v1`.** Porting markup into components invites "improvements". The rule is that `tokens.css` and `style.css` carry over byte for byte in step 1, `tools/contrast.py` keeps running, and a visual improvement (allowed by the decision) goes in its own PR with before/after screenshots, never inside a port step.
4. **Node becomes a build dependency** for `release` and `desktop-smoke`, on 3 OS and in `Cross` containers. `check` already sets up Node 22 for the UI tests, which satisfies Vite 8 (`^20.19 || >=22.12`). The committed `vue-ts/package-lock.json` already lists Rolldown's native packages for Linux, macOS (`@rolldown/binding-darwin-{arm64,x64}`) and Windows (`win32-{x64,arm64}-msvc`), so what remains unproven is installing and building on each of the three runners; the first step has to show that in CI. Estimated cost for the `check` job: locally `npm ci` took 1.6-3.2 s and the Vue build 1.6-2.5 s, so `[INFERENCE]` well under the 60 s budget even with a cold npm cache on the runner.
5. **Two runtimes during migration.** While steps 4-6 run, Vue and the legacy modules share the page. Shared state must go through one store module, or the PR #61 class of bug comes back.
6. **Frame-path performance.** Vue reactivity must not wrap the video, canvas or WebSocket objects. Step 6 is the one most likely to regress, so it comes last and has the measured gate.
7. **TypeScript 7 and `vue-tsc`.** The Vue template still pins TypeScript 6.0.3 because `vue-tsc` does not accept 7 yet; the other templates use 7.0.2. A TS upgrade waits for `vue-tsc`.
8. **The numbers are from one loaded machine** (load 5-16). Sizes and reproducibility repeat exactly; times do not, and first-frame times do not rank the options.

## What merges to `main`

Only this ADR. The measurement harness (`tools/frontend-bench/`, `tools/webview-probe/`, 12 lockfiles, about 620 KB) stays on branch `research/frontend-stack`, where every command above runs; the tag `adr-0001-evidence` marks the commit that produced the numbers, so the evidence survives edits to this file and deletion of the branch. Merging the harness would put toy dependencies under the repository's dependency alerts for no product benefit. The `tools/webview-probe` images move to `main` in migration step 1, as part of the PR gate.
