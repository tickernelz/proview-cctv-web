# Design

Version 2 (ZHA-4181, 2026-10-28). Zhafron opened a full redesign: Direction B
("Window") is the starting layout, no longer a contract. What binds instead are
the gates: AA contrast measured by `tools/contrast.py`, no horizontal scroll from
320 to 1920 px (`e2e/tests/responsive.spec.ts`), 44 px targets under a coarse
pointer, one icon family, motion that respects `prefers-reduced-motion`, and a
screenshot baseline for every visual change (`e2e/tests/*-snapshots/`). The
source of truth for every value is `apps/web/src/tokens.css`; this file says why.
The options considered for v2 are in `docs/design/redesign-v2/`; the component
layer decision is ADR 0002.

The layout keeps v1's idea: one window onto one camera. The picture is the page;
above it one line (the camera name, which is also the camera picker, and the
icons), below it one caption line and one row of controls.

## Principles

1. **The picture wins.** It is the largest and the brightest element on every
   screen: 72 % of a 1440×900 window on Live. Chrome that does not change what
   you see moves out of the way.
2. **Colour means a state.** Neutral graphite everywhere, and one meaning per
   colour: red only while recording or counting alarms, yellow only for data
   that is not fresh, green only for online or supported, blue for what is
   selected, pressed or primary (and the focus ring). v1 drew selection as ink
   on ink, so a pressed control and a hovered one looked the same; v2 gives
   selection its own colour.
3. **Say what is true, under the picture.** One caption names the camera, the
   state in words ("live, bottom lens", "frozen — the frame below is from
   21:39:58", the server's own refusal) and a monospace clock. Text goes on top of
   the video only in full screen.

## Colour

Graphite at hue 255 with chroma ≤ 0.008, written as the hex values the study
measured. The light page is `#dfe0e2`, not white, so the picture stays brighter
than the chrome.

| Token | Dark | Light | Role |
|---|---|---|---|
| `--bg` | `#0e1013` | `#f2f3f5` | page, top bar |
| `--bg-2` | `#171a1f` | `#ffffff` | inputs, menus, sheets, raised surfaces |
| `--bg-3` | `#21252b` | `#e5e7eb` | hover, skeleton, thumbnail placeholder |
| `--line` | `#3c414a` | `#b3b8c0` | input borders, table rules |
| `--line-soft` | `#2c3037` | `#c9cdd3` | row separators, top bar |
| `--ink` | `#eef0f3` | `#14171c` | main text |
| `--ink-2` | `#c4c9d0` | `#3b414a` | control labels, secondary text |
| `--ink-3` | `#9da4ae` | `#5a616b` | clock, hints, meta |
| `--stage` | `#030405` | `#030405` | behind the video, both themes |
| `--stage-ink` / `--stage-ink-2` | `#cfd1d3` / `#96989c` | `#d6d8da` / `#9c9fa2` | text drawn on the stage |
| `--accent` / `--accent-ink` | `#74b4ff` / `#061423` | `#0b5fbd` / `#ffffff` | primary button, pressed control, selected row, checked radio and switch |
| `--accent-soft` | accent at 14 % | accent at 10 % | fill behind a pressed or selected control |
| `--live` | `#fc665f` | `#b10d1a` | recording |
| `--badge` / `--badge-ink` | `#c9363c` / `#ffffff` | `#c4252b` / `#ffffff` | the alarm count on the bell |
| `--ok` | `#6cce8a` | `#176b39` | online dot, "supported" |
| `--warn` | `#edb95d` | `#875a04` | frozen, stale, offline, a failed tile |
| `--bad` | `#fb7172` | `#b9232d` | form errors, destructive actions |
| `--focus` | `#74b4ff` | `#0b5fbd` | the focus ring (same hue as the accent) |

Derived tokens are alpha over a listed colour, never a new hue: `--live-soft`
(live at 16 % / 10 %), `--stage-matte` (stage at 90 %, the move pad and the
full-screen line), `--stage-line` and `--stage-hover` (white at 14 % / 8 % on
the stage). No rule uses `color-mix()` or `oklch()`, so the colours do not
depend on what the WebView parses.

### Contrast

`python3 tools/contrast.py` parses `apps/web/src/tokens.css`, composites every alpha
token over its real background, and exits non-zero when a pair falls under its
floor; CI runs it in the `check` job. Floors: 4.5:1 for text, 3:1 for dots and the focus ring, 1.6:1 for `--line` and 1.4:1 for
`--line-soft` (a floor this project sets for separators; input edges do not
rely on the line alone because inputs sit on `--bg-2`). The tightest pairs it
reports, all passing:

| Pair | Dark | Light |
|---|---|---|
| ink-3 on page | 7.58 | 5.63 |
| warn on page | 10.61 | 5.41 |
| live on live-soft (recording control) | 5.34 | 5.39 |
| bad on page | 6.99 | 5.66 |
| accent-ink on accent (primary button) | 8.58 | 6.20 |
| badge-ink on badge (alarm count) | 5.15 | 5.76 |
| stage-ink-2 on the move pad over a white frame | 5.84 | 6.34 |
| focus ring on page / on stage | 8.81 / 9.49 | 5.59 / 3.31 |

76 of 76 pairs pass.

## Type

IBM Plex Sans and IBM Plex Mono, bundled as five latin-subset woff2 files in
`apps/web/public/fonts/` with `OFL.txt` (SIL OFL 1.1, © IBM Corp.), served by the server at
`/fonts/*` and so inside the desktop app too. `system-ui` would mean Segoe, SF
or Cantarell depending on the WebView; bundling keeps one face on every OS and
works offline.

Fixed px scale: 30 display / 21 heading / 17 title / 15 UI / 14 body / 13 small /
12 mono data and captions. Nothing renders under 12 px (v1 used 10-11 px in the
alarm sheet and the pad notes; `--t-11` now resolves to 12 px). No tracked
capitals; the only capitals are the wordmark. Mono only for machine data: clocks,
ids, addresses, error codes. Every clock, timestamp, duration and file size uses
tabular numerals, so a ticking clock does not shift its neighbours.

## Space, shape, elevation

Spacing on a 4 px grid, one token per value: `--s1` 4 · `--s2` 8 · `--s3` 12 ·
`--s4` 16 · `--s5` 20 · `--s6` 24 · `--s7` 32 · `--s8` 40 · `--s9` 56 (v1's 6, 10
and 14 px steps are gone). Tight inside a control (8), loose between
groups (16–24); the page edge is 20 px (16 below 480 px) and the stage runs to
it. Radius 6 (code chips), 8 (buttons, inputs, tiles), 12 (stage, menus, sheets);
pill only for switches, badges and table status. Three layers: the flat page,
`--bg-2` for inputs, menus and sheets, and a shadow only on what floats. No cards.

Fixed sizes: top bar 56, control row 64, control 36 (44 under
`pointer: coarse`, through `--hit`), menu row 48, pad key 40.

## Motion

160 ms for hover and press, 200 ms for a menu or sheet entering (opacity and
4 px), one standard easing `cubic-bezier(0.2, 0, 0, 1)`. A press moves the
control down 1 px, so a click is felt without a ripple. Motion only marks a state change: no page
entrance, no pulsing record dot. The loading line on the stage moves 1.2 s
linear. `prefers-reduced-motion: reduce` sets both duration tokens to 0, stops
the loading line and drops the menu animation; the microphone level keeps moving
because it is data.

## Screens

**Sign in.** Form on the left, an empty 16:9 stage on the right where the picture
will appear. Tabs Account / This network (arrow keys switch). The button keeps
its width and reads "Signing in…" while the request runs. A refusal marks the
password field, says who refused, and selects the password. A session the server
ended reads as what happened and when, notes that nothing records while signed
out, and "Sign in to resume" returns to the camera that was open. Settings opens
from here, before any session exists.

**Live.** The camera name with its status dot is the picker: a menu of every
camera (each device channel is one camera) with dots, numbers 1–9, Refresh and
Add a camera. Three icons on the right: every camera, Settings, theme. Under the
picture the caption (`role="status"`; only the camera name and the state words
are announced, the clock is outside the live region) and the control row:
Talk · Sound | Record · Snapshot on the left, Move · Lens · Quality | full screen
on the right. Controls the camera cannot do stay visible but disabled, with the
server's reason as their tooltip; the full evidence lives in Settings → This
camera.

**Stage states.** `loading` (a 2 px line and one sentence of what is awaited),
`playing` (only after a frame decoded), `stalled` (the last real frame stays,
the caption turns yellow with the time of that frame), `error` (title, the
server's answer in mono, Try again, and "Check the camera password" when the
answer is an authentication refusal), `offline` (the camera list reports it
offline). A socket that opens and sends nothing decodable becomes `error` after
six seconds.

**Lens.** A dual-lens camera sends one frame with both pictures stacked. "Both
lenses" shows it as sent; Top and Bottom crop it in the browser with no second
connection. The menu names each lens by role when the lens contract reports
`ptz`, by position otherwise, and its last line says whether the split was
reported or inferred.

**Move.** A pad over the bottom right of the stage on a 90 % stage matte, so its
keys read on a white frame. Arrow keys move the lens while it is open, Space
stops, presets are listed by name. It opens only when the capability report
marks movement supported for the moving lens.

**Every camera.** Up to four tiles, whole frame each (`object-fit: contain`,
never a crop). Each tile is its own stream and fails on its own, with the
reason and a Try again inside that tile. The selected camera is a 1.5 px ink
ring, not an accent. The row under the tiles is one sentence: "4 cameras · 3 live
· 1 offline".

**Full screen.** The stage edge to edge; with "Camera name and clock in full
screen" on, one small mono line at the bottom edge on a stage matte.

**Banner.** One line under the top bar, only for what affects the whole window or
the open camera: this computer lost its network, the open camera is offline in the
list, the camera list could not be read. Retry now is the only action.

**Cloud recordings.** A panel above the control row (Vue,
`src/live/RecordingsPanel.vue`), opened by Cloud. It opens on the camera on
screen with focus in Day; Escape or Close returns focus to Cloud. Load reads
"Loading…" with `aria-busy` and keeps focus while one request runs (a second
press does nothing). An empty day names the camera and the day; a failed load is
a `--bad` line with the warning icon and `role="alert"`, not grey text; a session
that ended goes back to sign-in.

**Alarm messages.** The bell in the top bar (Vue, `src/live/AlarmCenter.vue`)
counts alarms from the first check after sign-in, without opening anything. The
sheet sits in the same corner as Cloud recordings and replaces it: one corner
sheet at a time (`live.sheet` in the store). It opens with focus on Camera;
Escape or Close returns focus to the bell, which carries `aria-expanded`.
Camera and window ("Last 24 hours", "Last 7 days") change the list at once; a
check still out for the previous choice is dropped, so a list never shows under
a camera it does not belong to. Loading and empty states name the camera and
the window; a failed check is a `--bad` line with the warning icon and
`role="alert"`. The footer is "3 alarms · updated 22:16", with the vendor and
round-trip timing in its tooltip. Rows show the event, the time in tabular
figures, and the channel only for multi-channel cameras, counted from 1 like the
caption; a row without a picture shows the `no-picture` icon instead of an
empty box.

**Settings.** A screen, not a panel: sections Playback, This camera, Connection,
Configuration and Server on the left, one section on the right, "← Live" back to
the last camera. Playback (browser preferences, applied at once), This camera
(name, id, connection, lens layout, the capability table with the device's
evidence for every row, the white light), Connection (provider choice, the
server's fallback notice verbatim, sign out), Configuration (every server key,
grouped; a key read once at startup says "Needs a restart" under its label;
secrets render empty with "stored, type to replace"), Server (live checks of
health, account, camera probe and stream ticket). The save bar always shows the
state: "Everything saved", "Unsaved changes · 2 fields" with Discard and Save, or
"Saved. Restart the app to apply 1 field" with Restart on the desktop.

## Keyboard

1–9 camera n · G every camera / Live · F full screen · M sound · T held talk ·
R record · S snapshot · L lens Both → Top → Bottom · Q HD / SD · V move pad ·
arrows move the lens (pad open) or the tile focus (grid) · Space stop · ,
Settings · Esc closes the menu, then the pad, then full screen · ? the list,
as a sheet, not a modal. Letter keys do nothing while focus is in a text field.
In Settings: arrow keys walk the sections, Ctrl+S saves, Esc goes back when
nothing is unsaved. A menu returns focus to its trigger when it closes;
switching camera keeps focus where it was.

## Menus and the WebView

The lens and quality menus on Live are Reka UI `DropdownMenu` (Vue): they open
above the control row, aligned to the trigger's end, shift inside an 8 px
viewport margin, and keep `#lens-pick`, `#quality-pick`, `#lens-menu` and
`#quality-menu` as ids. Reka brings the same keys (arrows, Home/End, Escape and
Tab close and return focus). While one of them is open, the page's own keys
(Escape for the pad and the device sheet, arrows for pan and tilt, letter
shortcuts) step aside, so Escape closes only the menu. The menus below are
still the page's own.

Other menus use the Popover API when `HTMLElement.prototype.showPopover` exists
(light dismiss and top layer from the platform) and a `hidden`-toggle with an
outside-pointer handler otherwise; `<html data-popover>` records which path
ran. Both paths share the same roving focus (arrows wrap, Home/End, Escape and
Tab close and return focus). `tests/web/ui.test.mjs` runs the menu tests on both
paths. WebKitGTK 2.54 (Fedora 44) reports `showPopover`; the fallback was also
rendered there with the API deleted.

## Icons

One family: Lucide (`lucide-static` 1.63.0, ISC, licence in
`apps/web/public/licenses/`). `apps/web/icons.ts` reads the SVGs at build time
and writes them into the inline sprite of `index.html`, so the page keeps one
`<use href="#i-name">` per icon and loads no icon font or runtime. 24 px grid,
drawn at 20 px (16 px in menus), one 1.75 stroke. Each symbol has one meaning:
`history` is recorded playback on the device, `cloud` is cloud recordings, `bell`
is alarms, `cctv` is the device list; v1 drew Playback and Cloud with the same
clock and the bell as an emoji.

## Responsive

`e2e/tests/responsive.spec.ts` opens every main screen at 320×568, 568×320,
360×640, 640×360, 390×844, 844×390, 768×1024, 1024×768, 1280×800, 1440×900 and
1920×1080 and fails on any horizontal scroll or element past the edge.

| Width | Change |
|---|---|
| < 1024 | auto grid: 2 columns (1 below 640) |
| < 900 | sign-in becomes one column; the decorative stage is hidden |
| < 768 | settings sections become a wrapping row above the content |
| < 640 | settings rows stack label over control; tiles keep 16:9 and scroll; stream controls show icons only; the pan and tilt pad docks above the controls |
| < 480 | top bar tightens to 16 px edges, the camera name ellipsises, the meta is hidden; every sheet spans the width |

The gate walks every state a user reaches: both sign-in tabs, live, the pan and
tilt pad, the camera and quality menus, the alarm, cloud-recording and devices
sheets, playback, every camera, and each Settings section. Tables are the one
thing allowed to scroll sideways, inside their own box. Under `pointer: coarse`
(`e2e/tests/touch.spec.ts`, phone 390×844 and tablet 768×1024) every visible
button, link, input, select, tab and menu item in those states is at least
44×44.

## Focus and targets

A 2 px `--focus` ring with a 2 px gap, on `:focus-visible` only. Every control
is at least 36×36 (`--hit`), 44 under `pointer: coarse`. A "Skip to controls"
link is the first stop on Live.

## Not used, on purpose

Hero-centred sign-in, cards with shadows, emoji or Unicode glyphs as icons,
styles injected by scripts, gradients, tracked capital eyebrows, a coloured
left border as accent, blur, radius above 10 px, cream backgrounds, entrance
animation, empty-state illustrations, text over the video outside full screen,
numbers that were not measured.
