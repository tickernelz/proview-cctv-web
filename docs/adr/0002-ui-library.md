# ADR 0002: UI component layer and the v2 theme

- Status: accepted (ZHA-4181, 2026-10-28; choices delegated to the engineer by Zhafron at 16:17Z)
- Date: 2026-10-28
- Context issue: ZHA-4181 (refine UI/UX and responsiveness), on top of ADR 0001 (Vite + Vue 3)

## Context

ADR 0001 moved the page to Vite and Vue 3. Zhafron then asked for a component
library for Vue 3 (15:3xZ) and opened a full redesign of the theme (15:42Z).
The page must keep running in WebKitGTK 2.54 (desktop on Linux), WKWebView and
WebView2, so a library that needs a newer engine or brings its own runtime theme
is a cost, not only a size.

## Candidates

Bundle cost measured on 2026-10-28 on this box (numbers reported by Wira on
ZHA-4181): minified, then `gzip -9`. Reka UI and PrimeVue were bundled with
`esbuild --bundle --minify --format=esm`; Nuxt UI needs SFC compilation and its
Vite plugin, so it was built with `vite build`. Both are minified and gzipped,
so the comparison is fair in size, but not identical in method.

| Option | Components | JS gzip | vs Vue 3 alone (26,474 B) | CSS gzip |
|---|---|---:|---:|---:|
| Reka UI primitives (what shadcn-vue copies in) | Dialog, DropdownMenu, Tooltip, Tabs, Select, Switch, Slider | 94,265 B | +67.8 KB | own (Tailwind) |
| PrimeVue 5 + Aura preset | 12 | 146,427 B | +120.0 KB | theme as data URIs |
| Nuxt UI 4.12.0 | App, Button, Modal, DropdownMenu, Tooltip, Tabs, Select, Switch, Slider, Toast | 165,782 B | +139.3 KB | 25,902 B |
| Tailwind v4 alone (sample of the classes we use) | - | - | - | 2,823 B |

The awesome-vue "responsive" list Zhafron shared was sorted by what decides here,
not by popularity:

- Vue 2 only, unusable: buefy, iview-ui, BootstrapVue, fish-ui, heyui, AT-UI,
  Osiris UI, MDBootstrap (Vue 2), vue-material-adapter, Carvue, zircle-ui,
  BalmUI, Prefect Design.
- A visual language of its own, so a skin-over cost on top of the size:
  Vuetify (Material 3), Element Plus, Ant Design Vue, Naive UI, Vuestic,
  Qui-max, Inkline, Maz-UI, CoreUI, Oruga, Vexip, Stellar UI, flowbite-vue,
  @oneflowui/ui, BoldKit.
- Headless or copy-in, in line with this decision: Reka UI, shadcn-vue,
  Nuxt UI (styled on Reka + Tailwind), AgnosticUI; Inspira UI is an animation
  set, not a component system.

## Decision

1. **Components: shadcn-vue on Reka UI with Tailwind v4.** Components are copied
   into `apps/web/src/components/ui/` when the first Vue screen needs them,
   never installed as a themed runtime package. Reka gives focus management,
   keyboard handling and ARIA; the repository owns how they look. Only what a
   screen uses is copied, so the +67.8 KB floor is paid per component, not up
   front. Nuxt UI is the escape hatch; switch before the first component is
   copied in, never mix two systems.
2. **Icons: Lucide**, one family. Until screens are Vue components the legacy
   DOM uses an inline sprite generated at build time from `lucide-static`
   1.63.0 (`apps/web/icons.ts`), so no icon runtime ships; Vue components use
   `@lucide/vue` with the same names.
3. **Tokens: `apps/web/src/tokens.css` stays the only source of colour, space,
   radius and motion.** Tailwind reads it through `@theme` (PR #76); no
   component writes a raw colour.

## Theme decision

On 2026-10-28 Zhafron opened a full replacement of the visual language
("tema yang sekarang mau dirombak ulang dan di refine total ga masalah"). v2
keeps v1's layout idea (one window onto one camera) and replaces what made the
v0.3.5 build feel loose: mixed icon glyphs, ink-on-ink selection, scripts
injecting their own colours, 10-11 px text, 6/10 px spacing steps. Fidelity to
Direction B is replaced by these gates, all in CI:

- AA contrast for every token pair, `tools/contrast.py` (76 pairs);
- no horizontal scroll from 320 to 1920 px, `e2e/tests/responsive.spec.ts`;
- 44 px targets under a coarse pointer, `e2e/tests/touch.spec.ts`;
- screenshot baselines of every main screen in both themes, Chromium and WebKit,
  plus mobile and tablet;
- size budget against origin/main: JS gzip at most +80 KB, CSS at most +20 KB
  (measured in `docs/performance.md`).

## Consequences

- The first Vue screen pays for Reka + the copied components; the budget leaves
  12 KB of headroom over the measured 67.8 KB floor for seven primitives.
- Tailwind v4 raises the WebView floor to Chrome 111 / Safari 16.4 (PR #76,
  `docs/webview-matrix.md`).
- No theme package to upgrade; a Reka bump is an ordinary dependency PR with the
  e2e baselines as the check.
