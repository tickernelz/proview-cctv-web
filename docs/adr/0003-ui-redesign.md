# ADR 0003: UI redesign (Graphite with ruler timeline and status plate)

- Status: accepted (2026-10-30; Zhafron approved from the PDF mockup
  `~/re/proview-cctv/ux/mockup/PROVIEW-UI-Redesign-Mockup.pdf`)
- Date: 2026-10-30
- Base: `main` ddae4f0 (after v0.4.0)
- Builds on: ADR 0001 (Vite + Vue 3), ADR 0002 (shadcn-vue on Reka UI, Tailwind
  v4, Lucide, `tokens.css`)
- Design reference: `DESIGN.md` version 3

## Context

Zhafron's verdict on the v0.4.0 UI was "jelek dan tidak user-friendly" (ugly and
not user-friendly). The research that followed
(`~/re/proview-cctv/ux/RESEARCH.md`, 2026-10-30, basis `main` 8684d27) screenshot
every current screen against synthetic fixtures and studied 18 reference products
(UniFi Protect, Frigate, Verkada, Rhombus, Synology, Milestone and others). Its
section 1 ranks twelve problems; the top ones are about structure, not colour:

1. Live and recordings are separate worlds: Live, Cloud recordings and Playback
   are three surfaces, two of them overlays on the stage. Looking at "ten minutes
   ago" takes a native `<select>`, a date and a pick from a list of 30-minute
   files.
2. Playback is file-centred: a 44-row list of files with sizes in MB dominates,
   the day strip has no zoom and events are small colour marks.
3. The control row is 11 equal, unlabelled icons; disabled ones give their reason
   only in a tooltip.
4. Technical language leaks into the main status line.
5. The PTZ pad covers the picture, presets are underlined links, and there is no
   speed control.
6. A stalled stream is shown three times (caption, a yellow "frozen 4 s", the
   clock).
7. Settings is one flat list of 11 sections mixing app preferences, server
   configuration and the physical camera.

What the research found worth keeping: picture first, measured AA contrast
(`tools/contrast.py`), one icon family, explicit stage states, disabled controls
with the server's reason, and the keyboard map.

The shell itself was a set of Vue islands mounted into host elements of a static
`index.html` (`#live-top`, `#main`, `#settings`, `#signin-main`, ...), a leftover
of the step-by-step port in ADR 0001. A new IA that moves panels between screens
does not fit fixed host elements.

## Options

RESEARCH section 5 fixes one information architecture for every option: Live
(default) holds the timeline and playback, grid is a view mode of Live, Events
and Settings are the other two places, Settings is grouped by object. The options
differ in the visual system. The mockup draws both directions on the same frames
(1440x900, 1024x640, 390x844; dark and light for Live) plus hybrid frames.

| Option | What it is | Cost and risk (RESEARCH sections 6-7) |
|---|---|---|
| A "Graphite, refined" | Keep the ADR 0002 stack and principles; cool neutral hue 255, chroma <= 0.012 on surfaces; blue accent for selection only; IBM Plex Sans and Mono (already bundled); Lucide with labels; header 48, dock 56, side panel 320 | Lowest: the CI gates (contrast, responsive, touch, baselines) stay valid and the bundle grows only by the new primitives. Risk: it can look like a shadcn template. |
| B "Night desk" | A custom system: warm graphite (hue 60-80), teal signal colour, violet motion markers, Geist Sans and Mono, icons redrawn on a 20 px grid, physical-looking dock buttons, a ruler timeline and a status plate | Highest: custom components and icons, a new font, every e2e baseline changes, contrast recomputed. Gives the product an identity. |
| A + B | A's visual system plus B's two distinctive components: the ruler timeline and the status plate | A's cost plus two product components that are custom in any option anyway. |

## Decision

Zhafron chose the combination and four further points (2026-10-30):

1. **Visual direction: A + B.** Direction A "Graphite, refined" (shadcn-vue on
   Reka UI and Tailwind v4; cool neutral hue 255, chroma <= 0.012 on surfaces;
   blue accent only for selected, primary, the playhead and focus; red only for
   live, recording and alarm; yellow for motion events and warnings, separated by
   shape: an event marker is a 2 px tick, a warning a chip with an icon; IBM Plex
   Sans 400/500/600 and Plex Mono 400/500 bundled in `apps/web/public/fonts`;
   Lucide icons with labels; motion 160 ms hover and press, 200 ms panel, 240 ms
   chrome fade, one easing `cubic-bezier(0.2, 0, 0, 1)`; fixed sizes header 48,
   dock 56, side panel 320), plus Direction B's ruler timeline (104 px: ruler 20,
   track 48, filmstrip on hover) and status plate (a small plate top-left on the
   video: a dot and "LIVE" or "REC 14:32:10" in mono, on the stage colour at
   80 %).
2. **Dual-lens default: picture in picture.** The top lens large, the bottom lens
   in a small window, with a swap button.
3. **Timeline: a ruler.** It opens at a 24 h span; clicking an event starts
   playback 5 s before it.
4. **Desktop navigation.** Events and Settings are buttons on the right of the
   header. The Events side panel is open by default on windows >= 1280 px wide and
   closed on smaller ones; the toggle is persisted.
5. **Dock.** Labelled main actions Talk, Sound, Snapshot, Record and Move;
   everything else under More. Click-to-aim works only while Move is on. PTZ
   speed keeps the app's three levels Slow / Normal / Fast (server API
   `speed=slow|normal|fast`), default Normal; the mockup's 1-5 scale is not used.

Tokens, sizes, breakpoints and the IA are in `DESIGN.md` version 3.

## Why

1. **A + B.** The complaint was about structure and hierarchy, not palette
   (RESEARCH section 1, problems 1-8). A keeps the ADR 0002 stack, the CI gates
   and the bundle budget, so the risk is low; but A alone can end "tidy but
   generic", and the owner missed an identity. The ruler timeline and the status
   plate supply it exactly where it carries meaning: the timeline is the new way
   into recordings and has to be custom in any option, and the plate states live
   or recorded on the picture itself. B's warm palette, Geist and redrawn icons
   would cost every baseline and a new contrast table for no gain in structure.
2. **Picture in picture.** Both lenses stay visible, the picture is not split in
   half, and the swap is one press. The browser crops one stream, so there is no
   second connection.
3. **Ruler at 24 h, play 5 s before.** A day is the unit people ask about ("what
   happened today"), and an event marker is the start of the motion, so the 5 s
   before it show what led to it. Time replaces the file list as the way in.
4. **Events on the right, open at >= 1280.** At 1280 px a 320 px column still
   leaves the stage the larger part of the window; below that it would squeeze
   the picture, so it overlays and starts closed. Persisting the toggle respects
   a user who closed it.
5. **Five labelled actions.** These are the frequent actions (RESEARCH section 1,
   problem 3: Snapshot and Talk weighed the same as Quality). Labels make the
   dock readable without tooltips. Click-to-aim only under Move stops a click on
   the picture from moving the camera by surprise. Three speeds match what the
   server already accepts, so the UI does not invent a scale the API cannot
   carry.

## Consequences

- **One root app.** The islands go: `apps/web/index.html` keeps only `#app` (with
  the pre-JS boot skeleton) and the icon sprite; `src/main.ts` mounts one
  `App.vue` from `src/shell/`, which renders the sign-in gate, the header, Live,
  the Events host, Settings, the phone tab bar, the toaster and the tooltip
  provider. A hash router owns `#/live`, `#/live/grid`, `#/live/playback`,
  `#/events` and `#/settings/<section>`. Existing screens move in as children
  as-is; the redesign of each happens in its slice.
- **Frozen shared files and five slices.** After the foundation PR the shared
  files (tokens, Tailwind mapping, base styles, shell, `components/ui`, icons,
  store, app state modules, API client, `tools/contrast.py`, shared e2e helpers
  and layout specs, CI, package files) change only through the integrator. Five
  slices then run in parallel with disjoint files: live-stage, timeline-playback,
  events, settings, signin-states-grid-mobile.
- **Baselines regenerated in the CI container.** Every screenshot changes. The
  integrator regenerates the visual, responsive and touch snapshots once, inside
  the image the `e2e` job runs in (`mcr.microsoft.com/playwright:v1.64.1-noble`,
  `.github/workflows/ci.yml`), so local font rendering does not leak into the
  baselines. Slices regenerate only the snapshots of the specs they own.
- **Contrast table grows.** `tools/contrast.py` now checks 57 pairs per theme,
  114 in total (v2 checked 76), including the timeline track, the status plate
  over a white frame and the unread badge; all pass on 2026-10-30. Two research
  tokens (`--line-soft`, `--rec-bar`) were changed before shipping because they
  failed their floors; `DESIGN.md` has the numbers.
- **Budget gates from ADR 0002 still apply.** JS gzip at most +80 KB and CSS at
  most +20 KB against the pre-Vue baseline, measured as in `docs/performance.md`.
  New primitives are copied into `components/ui`, never installed as a themed
  package.
- **Not changed:** the stack (ADR 0001, 0002), the WebView floor (Chrome 111 /
  Safari 16.4, `docs/webview-matrix.md`), the keyboard map, the touch and
  responsive gates, and the rule that a number in the docs is measured.
