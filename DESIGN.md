# Design

Version 3 (2026-10-30, ADR 0003). Zhafron approved a combination from the PDF
mockup (`~/re/proview-cctv/ux/mockup/PROVIEW-UI-Redesign-Mockup.pdf`): the visual
system of Direction A "Graphite, refined" (shadcn-vue on Reka UI and Tailwind v4,
the ADR 0002 stack) plus two components from Direction B "Night desk": the ruler
timeline and the status plate. The research behind it is
`~/re/proview-cctv/ux/RESEARCH.md` sections 5-9; the decision and its reasons are
ADR 0003.

What binds are the gates, unchanged from v2: AA contrast measured by
`tools/contrast.py`, no horizontal scroll from 320 to 1920 px
(`e2e/tests/responsive.spec.ts`), 44 px targets under a coarse pointer
(`e2e/tests/touch.spec.ts`), one icon family, motion that respects
`prefers-reduced-motion`, and a screenshot baseline for every visual change
(`e2e/tests/*-snapshots/`). The source of truth for every value is
`apps/web/src/tokens.css`, mapped to Tailwind utilities in
`apps/web/src/tailwind.css`; this file says why.

The structure changes, not the idea: one window onto one camera. Live, recorded
video and events are one surface. The header names the camera; the stage shows
it; a dock of labelled actions and a ruler timeline sit under it; events sit on
the right on wide windows.

## Principles

1. **The picture wins.** The stage is the largest and the brightest element on
   every screen. Chrome has fixed sizes (header 48, dock 56, timeline 104, side
   panel 320) so the picture gets the rest, and actions that are not used often
   go under More.
2. **Colour means a state.** Neutral graphite at hue 255 on every surface, and
   one meaning per colour: blue for selected, primary, the playhead and the focus
   ring; red only for live, recording and alarms; yellow for motion events and
   for warnings, told apart by shape (an event is a 2 px tick, a warning is a chip
   with an icon); green only for online. Colour never carries a state alone: a
   word or a shape goes with it.
3. **Time is the axis.** Live and recordings are one timeline. The playhead at
   the right edge is live; anywhere else is playback. Events are points on that
   axis, and opening one plays from 5 s before it.
4. **Say what is true, in two layers.** A short human sentence first ("Reconnecting.
   Last frame 18:56:29."), the server's own words in a mono Details block under
   it. Text never points at a tooltip for the reason.

## Colour

OKLCH is the source (RESEARCH section 6); `tokens.css` ships the hex the source
converts to. No authored rule uses `oklch()` or `color-mix()`: older WebViews
parse hex and `rgb()` alpha, and `tools/contrast.py` measures what it can parse.
Derived colours come from tokens (`-soft`, `--scrim`, `--stage-*`), not from
Tailwind opacity modifiers, so every colour the page draws is measured. Tailwind's
default palette is reset (`--color-*: initial`), so no raw colour utility exists.

| Token | Dark OKLCH | Dark hex | Light OKLCH | Light hex | Role |
|---|---|---|---|---|---|
| `--bg` | 0.165 0.006 255 | `#0d0e11` | 0.970 0.003 255 | `#f4f5f7` | page, header |
| `--bg-2` | 0.205 0.008 255 | `#15171b` | 1 0 0 | `#ffffff` | side panel, menu, input |
| `--bg-3` | 0.250 0.009 255 | `#1f2226` | 0.930 0.005 255 | `#e6e8eb` | hover, skeleton, timeline track |
| `--line` | 0.370 0.012 255 | `#3c4046` | 0.790 0.010 255 | `#b6bbc1` | input border, strong divider |
| `--line-soft` | 0.305 0.010 255 | `#2c2f34` | 0.855 0.008 255 | `#ccd0d5` | row divider |
| `--ink` | 0.955 0.004 255 | `#eef0f3` | 0.205 0.010 255 | `#14171c` | main text |
| `--ink-2` | 0.830 0.008 255 | `#c4c8cc` | 0.370 0.012 255 | `#3c4046` | control label |
| `--ink-3` | 0.715 0.012 255 | `#9ea4ab` | 0.480 0.014 255 | `#585e66` | meta, clock |
| `--stage` | 0.080 0.003 255 | `#010202` | same | `#010202` | behind the video |
| `--accent` / `--focus` | 0.740 0.130 250 | `#67b0f9` | 0.500 0.160 255 | `#0961bb` | selected, primary, playhead, focus ring |
| `--accent-ink` | 0.180 0.040 250 | `#031222` | 1 0 0 | `#ffffff` | text on accent |
| `--live` | 0.680 0.190 25 | `#f75d59` | 0.470 0.180 25 | `#a9131f` | LIVE, recording, alarm marker |
| `--event` | 0.800 0.130 80 | `#e9b452` | 0.560 0.120 75 | `#9d6800` | motion marker on the timeline |
| `--ok` | 0.780 0.140 150 | `#6fd087` | 0.470 0.120 150 | `#146d34` | online |
| `--warn` | 0.820 0.130 80 | `#f0ba59` | 0.500 0.110 75 | `#875800` | reconnecting, stale |
| `--bad` | 0.690 0.170 22 | `#f2696a` | 0.500 0.180 25 | `#b32228` | error, destructive |
| `--rec-bar` | 0.550 0.020 255 | `#6a727d` | 0.600 0.020 255 | `#78818c` | continuous recording segment |

`--badge` / `--badge-ink` (`#c9363c` / `#ffffff` dark, `#c4252b` / `#ffffff`
light) draw the unread count on the Events button.

### Changed from the research palette

Two tokens of RESEARCH section 6 failed a floor and were changed before they
shipped: four lightness values in all. The ratios below come from converting the
research OKLCH to sRGB hex (a throwaway script on 2026-10-30; the shipped values
convert to exactly the hex in `tokens.css`, which checks the conversion) and the
WCAG ratio against the same background `tools/contrast.py` uses.

| Token | Research L | Ratio | Shipped L | Ratio (`contrast.py`) | Floor |
|---|---|---|---|---|---|
| `--line-soft` dark, on page | 0.290 | 1.37 | 0.305 | 1.44 | 1.4 |
| `--line-soft` light, on page | 0.870 | 1.35 | 0.855 | 1.42 | 1.4 |
| `--rec-bar` dark, on the timeline track | 0.420 | 1.89 | 0.550 | 3.28 | 3.0 |
| `--rec-bar` light, on the timeline track | 0.780 | 1.63 | 0.600 | 3.22 | 3.0 |

`--line-soft` is the only line between rows, so it has to read; the recorded
segment is the one thing that tells "there is video here" from "there is not", so
it is held to the 3:1 floor for graphics, not to the separator floor.

### Derived tokens

Alpha over a listed colour, never a new hue:

| Token | Dark | Light | Use |
|---|---|---|---|
| `--accent-soft` | accent at 14 % | accent at 10 % | fill behind a pressed or selected control |
| `--live-soft` | live at 16 % | live at 10 % | fill behind Record while recording |
| `--scrim` | stage at 60 % | ink at 40 % | behind modal dialogs and sheets |

Shared by both themes, because the stage is dark in both:

| Token | Value | Use |
|---|---|---|
| `--stage-ink` / `--stage-ink-2` | `#d6d8da` / `#a9acb0` | text drawn on the stage |
| `--stage-live` | `#ff6f69` | live dot on the status plate |
| `--stage-warn` | `#f0ba59` | reconnecting chip, warning icon on the stage |
| `--stage-matte` | stage at 80 % | status plate, controls drawn on the video |
| `--stage-line` / `--stage-hover` | white at 14 % / 8 % | lines and hover on the stage |

Shadows: `--shadow-menu` and `--shadow-panel` (see Elevation).

### Contrast

`python3 tools/contrast.py` parses `apps/web/src/tokens.css`, composites every
alpha token over its real background (a stage token over a white video frame, the
worst case for text on the picture), and exits non-zero when a pair falls under
its floor. CI runs it in the `check` job. Floors: 4.5:1 for text, 3:1 for
graphics (dots, markers, the playhead, the focus ring), 1.6:1 for `--line` and
1.4:1 for `--line-soft` (a floor this project sets for separators; inputs do not
rely on the line alone because they sit on `--bg-2`).

Run on 2026-10-30 in the `ui/foundation` worktree: 57 pairs per theme, 114 in
total, exit 0, last line "all 114 pairs clear their floor". The tightest pairs,
by margin over their floor:

| Pair | Floor | Dark | Light |
|---|---|---|---|
| line-soft on page | 1.4 | 1.44 | 1.42 |
| line on bar (input border) | 1.6 | 1.72 | 1.93 |
| line on page | 1.6 | 1.85 | 1.77 |
| recorded segment on timeline track | 3.0 | 3.28 | 3.22 |
| focus ring on stage | 3.0 | 9.06 | 3.40 |
| event marker on timeline track | 3.0 | 8.45 | 3.88 |
| accent playhead on timeline track | 3.0 | 6.96 | 4.97 |
| live marker on timeline track | 3.0 | 5.07 | 6.11 |
| stage-live dot on status plate over a white frame | 3.0 | 4.55 | 4.55 |
| live on live-soft over page (recording control) | 4.5 | 5.09 | 5.77 |
| badge-ink on badge (unread count) | 4.5 | 5.15 | 5.76 |
| ink-3 on hover (timeline hour labels) | 4.5 | 6.35 | 5.33 |
| stage-ink-2 on status plate over a white frame | 4.5 | 5.43 | 5.43 |
| accent on page | 4.5 | 8.41 | 5.60 |
| warn on page | 4.5 | 10.91 | 5.62 |
| accent-ink on accent (primary button) | 4.5 | 8.22 | 6.11 |
| ink-3 on page | 4.5 | 7.68 | 6.00 |

A new token or a new pair goes into `tools/contrast.py` (a frozen file, so
through the integrator) in the same change that uses it.

## Type

IBM Plex Sans 400/500/600 and IBM Plex Mono 400/500, bundled as five
latin-subset woff2 files in `apps/web/public/fonts/` with `OFL.txt` (SIL OFL 1.1,
(c) IBM Corp.), served by the server at `/fonts/*` and so inside the desktop app
too. `system-ui` would mean Segoe, SF or Cantarell depending on the WebView;
bundling keeps one face on every OS and works offline.

| Token | px | Use |
|---|---|---|
| `--t-12` | 12 | captions, mono data, dock labels, timeline hours |
| `--t-13` | 13 | small text, meta |
| `--t-14` | 14 | body |
| `--t-15` | 15 | control labels |
| `--t-17` | 17 | titles |
| `--t-21` | 21 | headings |
| `--t-28` | 28 | display, sign-in only |

Line height 1.4 for body (`--lh-body`), 1.2 for headings (`--lh-head`). Nothing
renders under 12 px. No tracked capitals and no uppercase eyebrows. Mono
(`--f-mono`) only for machine data: clocks, the status plate, timeline hours, ids,
addresses, error codes and Details. Every clock, timestamp, duration and size uses
tabular numerals, so a ticking clock does not shift its neighbours.

## Spacing and fixed sizes

Spacing on a 4 px grid, one token per value: `--s1` 4, `--s2` 8, `--s3` 12,
`--s4` 16, `--s5` 20, `--s6` 24, `--s7` 32, `--s8` 40, `--s9` 56. Tight inside a
control (8), loose between groups (16-24).

| Token | px | What |
|---|---|---|
| `--header` | 48 | header |
| `--dock` | 56 | dock under the stage |
| `--timeline` | 104 | ruler timeline: ruler 20 + track 48, filmstrip on hover |
| `--panel` | 320 | Events side panel, docked or overlay |
| `--rail` | 240 | camera rail |
| `--tabbar` | 56 | bottom tab bar under 640 px |
| `--chip` | 24 | status plate and status chips |
| `--hit` | 36 | control height; 44 under `pointer: coarse`; 32 in the compact density at <= 1024x640 |

Event marker 2x12 px; event thumbnail 96x54 px.

## Radii

`--r-sm` 6 (chips, the status plate), `--r-md` 8 (buttons, inputs, tiles),
`--r-lg` 12 (stage, panels, menus). Pill only for switches, the unread badge and
status dots. Radius 12 is for containers, not for every element.

## Elevation

Flat surfaces separated by background steps (`--bg`, `--bg-2`, `--bg-3`) and
lines. A shadow only on what floats: `--shadow-menu` for menus, popovers and
tooltips; `--shadow-panel` for dialogs, sheets and the Events panel when it
overlays the stage. `--scrim` sits behind modal dialogs and sheets, never behind
the non-modal Events panel. The docked Events panel is a column with a
`--line-soft` edge and no shadow. No blur.

## Iconography

One family: Lucide (`lucide-static` 1.63.0, ISC, licence in
`apps/web/public/licenses/`). `apps/web/icons.ts` maps sprite ids to Lucide files
and writes them at build time into the inline sprite of `index.html`;
`src/components/Icon.vue` draws one `<use href="#i-<id>">`. No icon font, no
icon runtime. 24 px grid drawn at 20 px with a 1.75 stroke (`.i` in
`src/styles/base.css`), 16 px in menus and dense rows (`small`). Icons are
`aria-hidden`; the label or `aria-label` carries the name. Each symbol has one
meaning.

The dock shows every icon with a 12 px label; an icon alone is allowed only
where a tooltip and `aria-label` name it and space forbids a label (header icon
buttons, the phone dock).

Key set (RESEARCH section 6):

| Action | Lucide | Sprite id today |
|---|---|---|
| Talk | `mic` | `mic` |
| Sound on / off | `volume-2` / `volume-x` | `speaker` / `speaker-off` |
| Snapshot | `camera` | `camera` |
| Record | `circle-dot` | `record` |
| Move | `move` | `move` |
| Lens | `layers` | not in the sprite yet |
| Quality | `gauge` | not in the sprite yet |
| Full screen | `maximize` | `expand` |
| More | `ellipsis` | `more` |
| Events | `bell` | `bell` |
| Grid | `layout-grid` | `grid` |
| Settings | `settings` | `gear` |
| Back to live | `radio` | not in the sprite yet |
| Offline | `wifi-off` | `disconnect` |
| Warning, error | `triangle-alert` | `warning` |
| Done, supported | `check` | `check` |

The status dot is CSS, not an icon. Adding a sprite id changes `icons.ts`, a
frozen file, so it goes through the integrator.

## Motion

| Token | ms | Use |
|---|---|---|
| `--dur-1` | 160 | hover, press |
| `--dur-2` | 200 | panel or menu entering: opacity and translateX 8 px |
| `--dur-3` | 240 | chrome auto-hide fade |

One easing, `--ease` `cubic-bezier(0.2, 0, 0, 1)`. Motion marks a state change
only: no page entrance, no pulsing record dot. The playhead moves linearly in real
time because it is data. `prefers-reduced-motion: reduce` sets every duration to
0; the playhead and the microphone level keep moving.

## Components

### `apps/web/src/components/ui/`

shadcn-vue style: copied in, built on Reka UI primitives, styled only with token
utilities. Frozen after the foundation PR.

| Component | Purpose and variants |
|---|---|
| button | One shape for every text action. `variant` primary (accent fill), secondary (outlined, the default), ghost (no fill until hover), danger (`--bad`); `size` sm, md (`--hit`), lg; optional `icon` / `iconEnd`, `block`. `loading` keeps the width, sets `aria-busy` and ignores further presses. |
| icon-button | Square button with one icon. `label` is required and becomes both `aria-label` and the tooltip; optional `shortcut` shows in the tooltip. `variant` ghost or secondary, `size` sm or md; `pressed` uses `--accent-soft` and `--accent`. |
| select | Reka Select replacing native `<select>`; same height and border as an input. |
| switch | On/off setting that applies at once; checked is `--accent`. |
| slider | Continuous values (volume, the start position); value shown in tabular figures. |
| tabs | Peer views of one object (sign-in Account / This network, Events filters). Arrow keys switch. |
| dropdown-menu | Menus from a trigger: camera switcher, More, lens, quality. Roving focus, Home/End, wrap, Escape and Tab close and return focus. |
| dialog | Modal confirmation or short form over `--scrim`; focus trapped and returned. |
| sheet | Panel from an edge: the Events overlay under 1280 px, phone sheets. `--shadow-panel`. |
| tooltip | Names an icon-only control. Never holds the only copy of a reason. |
| badge | Short count or tag: unread count on Events, Restart tag in Settings. `tone` neutral, accent, live, ok, warn, bad; warn always shows its icon, so it never reads as a motion event. |
| status-dot | Dot with its word. `status` online, offline, unknown, live, warn, recording; `label` is required and is the accessible name (`role="img"`). |
| card | One bordered `--bg-2` group (a Settings group, the add-a-camera panel). No shadow, never a card inside a panel. |
| field | Label, control, hint and error as one unit (`label`, `hint`, `error`, `disabledReason`, `required`); its slot passes `id`, `describedBy`, `invalid` and `required` to the control. The error is `--bad` text with the warning icon and `aria-invalid`; `disabledReason` puts the reason for a disabled control on screen. |
| toast | Outcome of an action that has no place on screen (snapshot saved, clip downloaded). `toast({ title, description, tone, action, duration })` with `tone` neutral, ok, warn, bad; `dismiss(id)`; one `<Toaster>` in the shell. |
| empty-state | Title, one sentence, at most one action; no illustration. |
| skeleton | `--bg-3` blocks in the shape of the content while it loads; still under reduced motion. |
| segmented-control | One choice among two to four peers shown at once: Single / Grid, PTZ speed Slow / Normal / Fast, grid columns. Reka ToggleGroup (single); `options` of `{ value, label, icon, disabled }`, a required `label` that names the group, `size` sm or md, `iconOnly`; `disabled` disables the whole group. |
| kbd | A key in the keyboard sheet and in tooltips, mono. |

### Product components

**Status plate.** Top-left on the video, `--chip` high, `--r-sm`, on
`--stage-matte`: a dot and a word in mono. `LIVE` with the `--stage-live` dot
when the playhead is live; `REC 14:32:10` (the time of the frame on screen) in
playback. It is the one piece of text on the picture outside full screen,
besides stage state messages, and its contrast is measured over a white frame.

**Ruler timeline.** 104 px under the dock: a 20 px ruler with hour labels in
mono `--ink-3`, a 48 px track on `--bg-3` with `--rec-bar` segments where
recording exists, event markers as 2x12 px ticks (`--event` for motion, `--live`
for alarms), and the playhead in `--accent`. A filmstrip of event thumbnails
(96x54) shows on hover. It opens at a 24 h span. Clicking an event starts
playback 5 s before it. While the playhead is in the past, Back to live returns
to the live edge. An empty day is a sentence on the track ("No recording on 30
Oct."), not an empty bar.

**Dock.** 56 px under the stage. Labelled main actions Talk, Sound, Snapshot,
Record and Move; every other action sits under More. A control the camera cannot
use right now stays in place, disabled, with its reason in words on screen.

**PTZ pad.** Shown on the stage while Move is on; click-to-aim works only while
Move is on, so a click on the picture never moves the camera by surprise. Speed is
the app's three levels Slow / Normal / Fast (server API `speed=slow|normal|fast`),
default Normal, as a segmented control; the mockup's 1-5 scale is not used. Arrow
keys move, Space stops, presets are listed by name. It is offered only when the
capability report marks movement supported for the moving lens.

**Dual-lens picture in picture.** A dual-lens camera sends both pictures stacked
in one frame. The default view is picture in picture: the top lens large, the
bottom lens in a small window, with a swap button; the browser crops the one
stream, no second connection.

**Events panel.** 320 px on the right. Rows: thumbnail, event kind in words, the
camera, the time in tabular mono. Opening a row moves the timeline and starts
playback 5 s before the event. Docked and open by default at >= 1280 px, an
overlay sheet below; full page at `#/events`.

## States

Every screen draws these, in these words:

| State | What shows |
|---|---|
| Loading | Skeleton: header text, dock and timeline track as `--bg-3` blocks; a 2 px line and one sentence of what is awaited on the stage. |
| Playing | Only after a frame decoded; the status plate reads `LIVE` or `REC <time>`. |
| Stalled | The last real frame stays; a `--stage-warn` chip with the warning icon: "Reconnecting. Last frame 18:56:29." One representation, not three. |
| Offline | The last snapshot dimmed and "Offline since <time>" with the offline icon. |
| Error | Title, one sentence, a mono Details block with the server's answer, and Try again. |
| Empty: no camera | The add-a-camera panel in place of the stage. |
| Empty: no recording that day | A sentence on the timeline track naming the day. |
| Not supported | The camera cannot do it at all: the action is not offered in the dock, and Settings > Camera shows one row saying so, with the evidence under Details. |
| Disabled | The camera can do it but not now (no session, lens not movable, already busy): the control stays visible and disabled, and its reason is on screen, not only in a tooltip. |

A socket that opens and sends nothing decodable becomes Error after six seconds.

## Responsive

Breakpoints in `tailwind.css`: sm 640, md 1024, lg 1280, xl 1440.

| Width | Layout |
|---|---|
| < 640 (mobile) | Header 48 (camera name and status), video full width, bottom tab bar 56 (Live, Events, Settings). Events is the `#/events` route; no side panel. Grid is one column. |
| 640-1023 (tablet) | Header; Events opens as a 320 px overlay sheet from the right over the stage. |
| 1024-1279 (desktop small) | Same overlay panel, closed by default. |
| >= 1280 (wide) | Events docked as a 320 px column right of the main region, open by default. |

The Events toggle is persisted in `localStorage` key `pvw_events_panel`
(`"open"` or `"closed"`). At <= 1024x640 the compact density sets `--hit` to
32. Under `pointer: coarse` every target is 44 px.

`e2e/tests/responsive.spec.ts` opens every main screen at 320x568, 568x320,
360x640, 640x360, 390x844, 844x390, 768x1024, 1024x768, 1280x800, 1440x900 and
1920x1080 and fails on any horizontal scroll or element past the edge. Tables
are the one thing allowed to scroll sideways, inside their own box.
`e2e/tests/touch.spec.ts` checks 44x44 targets on a 390x844 phone and a
768x1024 tablet.

## Information architecture

| Place | Route | What |
|---|---|---|
| Live | `#/live` (default) | One camera, dual-lens picture in picture, dock, PTZ under Move, the ruler timeline with playback in it. |
| Live, grid | `#/live/grid` | Every camera as tiles: a view mode of Live, switched by the Single / Grid segmented control in the header, not a page. |
| Live, playback | `#/live/playback` | The playback screen until the timeline slice folds it into the timeline. |
| Events | `#/events` | Full-page review; on wide windows also the docked side panel. |
| Settings | `#/settings/<section>` | Three groups, below. |
| Sign in | gate | Before everything; Settings stays reachable from it. |

Settings groups and their section ids:

| Group | Sections |
|---|---|
| Camera | `camera`, `device`, `image`, `alarm`, `system` |
| App | `playback`, `config` |
| Account & server | `account`, `connection`, `server` |

The save bar stays the one place for the outcome of a save: "Everything saved",
"Unsaved changes - N fields" with Discard and Save, a refused save as a `--bad`
alert with the server's reason, and the restart prompt after a save.

Desktop header, left to right: brand, camera switcher with its status, Single /
Grid, then on the right Events (with the unread count), Settings, theme and the
keyboard list. A banner under the header carries problems that affect the whole
window or the open camera. Under 640 px the bottom tabs Live / Events / Settings
replace the right-hand buttons.

## Keyboard

1-9 camera n - G Single / Grid - F full screen - M sound - T held talk - R record -
S snapshot - L lens - Q HD / SD - V move pad - arrows move the lens (pad open) or
the tile focus (grid) - Space stop - , Settings - Esc closes the menu, then the
pad, then full screen - ? the list, as a sheet, not a modal. Letter keys do
nothing while focus is in a text field. In Settings: arrow keys walk the
sections, Ctrl+S saves, Esc goes back when nothing is unsaved. A menu returns
focus to its trigger when it closes; switching camera keeps focus where it was.
While a Reka menu is open, the page's own keys step aside, so Escape closes only
the menu.

## Menus and the WebView

Menus, selects, tooltips and sheets are Reka primitives from `components/ui/`;
they render inside the page (teleported to `body`), not as native windows, so
they look and behave the same in Chrome, WebKitGTK, WKWebView and WebView2. They
open aligned to the trigger and shift inside an 8 px viewport margin. The page
floor is Chrome 111 / Safari 16.4 because of Tailwind v4
(`docs/webview-matrix.md`).

## Focus and targets

A 2 px `--focus` outline with a 2 px offset, on `:focus-visible` only (rows in
menus draw it inset). Every control is at least `--hit` high: 36 px, 44 under
`pointer: coarse`, 32 in the compact density. A "Skip to controls" link is the
first stop on Live. Status is never colour alone: the dot has a word, the event
tick has a kind in the Events row, a warning has an icon.

## Not used, on purpose

Hero-centred sign-in; default shadcn look (cards with shadows, a card inside a
panel, radius 12 on every element); emoji or Unicode glyphs as icons; icons
without a name; styles injected by scripts; gradients; tracked capital eyebrows;
a coloured left border as accent; blur; cream backgrounds; entrance animation;
empty-state illustrations; text over the video other than the status plate, stage
state messages and the full-screen line; a reason that lives only in a tooltip;
file lists as the way into recordings; the 1-5 PTZ speed scale; Direction B's
warm palette, teal signal colour, Geist and redrawn icons; numbers that were not
measured.

## File ownership for the next phase

Five slices run in parallel after the foundation PR. Shared files change only
through the integrator: a slice that needs one changed (a new store field, a new
icon, a new contrast pair) asks the integrator instead of editing it.

Frozen after the foundation PR, changed only through the integrator:
`apps/web/index.html`, `src/main.ts`, `src/tokens.css`, `src/tailwind.css`,
`src/styles/base.css`, `src/shell/**`, `src/components/ui/**`,
`src/components/Icon.vue`, `apps/web/icons.ts`, `src/live/store.ts` (slices may
ask the integrator for new fields), `src/app/state.ts`, `src/app/shell.ts`,
`src/app/view.ts`, `src/app/chrome.ts`, `src/app/keys.ts`, `src/app/boot.ts`,
`src/lib/api.ts`, `tools/contrast.py`, `e2e/tests/app.ts`, `e2e/tests/layout.ts`,
`e2e/tests/visual.spec.ts` (and its snapshots, regenerated once by the
integrator), `e2e/tests/shell.spec.ts`, `chrome.spec.ts`, `menus.spec.ts`,
`responsive.spec.ts`, `touch.spec.ts` (and their snapshots),
`tests/web/cdp.mjs`, `tests/web/ui.test.mjs`, `.github/workflows/**`,
`package.json` and its lockfile.

| Slice | Owns |
|---|---|
| 1. live-stage | `src/live/LiveMain.vue`, `LiveBar.vue`, `ImageLive.vue`, `FisheyeOverlay.vue`, `PtzEditor.vue`, `fisheye.ts`, `talk.ts`, new `src/live/stage/**`; `src/app/stage.ts`, `controls.ts`, `lens.ts`, `media.ts`, `caption.ts`, `dst.ts`, `format.ts`; `src/lib/stream-player.ts`, `hevc.ts`; `src/styles/live.css`; e2e live, stage, controls, talk, talk-race, lens, fisheye, live-audio; tests/web snapshot, stream-orphan, stream-misconfigured, live-late-viewer, hevc-fallback, hevc-verdict |
| 2. timeline-playback | `src/live/PlaybackScreen.vue`, `PlaybackStrip.vue`, `PlaybackRows.vue`, `PlaybackCalendar.vue`, `RecordingsPanel.vue`, `playback-audio.ts`, `playback-clips.ts`, new `src/live/timeline/**`; `src/styles/playback.css`; e2e playback, playback-audio, recordings |
| 3. events | `src/live/AlarmCenter.vue`, new `src/events/**`; `src/styles/events.css`; e2e alarms; tests/web alarms-pictures |
| 4. settings | `src/live/SettingsScreen.vue`, `DeviceSettings.vue`, `AlarmSettings.vue`, `ImageSettings.vue`, `OsdSettings.vue`, `RecordSettings.vue`, `CardFormat.vue`, `SystemAbout.vue`, `AccountPanel.vue`, `DevicesSheet.vue`, new `src/settings/**`; `src/styles/settings.css`; e2e settings, device-settings, alarm-settings, image-settings, system-about, device-admin, devices, add-camera; tests/web device-settings, restart, settings-boot, desktop-settings |
| 5. signin-states-grid-mobile | `src/live/SignInScreen.vue`, `AccountFlows.vue`, `account-flows.ts`, `AddCamera.vue`, `GridView.vue`, `GridTile.vue`, `grid-clock.ts`, new `src/states/**`; `src/styles/signin.css`, `grid.css`; e2e signin, grid; tests/web grid |

Paths under `src/` are relative to `apps/web/`.
