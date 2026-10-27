# Design

Direction B, "Window": the app is one window onto one camera. The picture is the
page; above it sits one line (the camera name, which is also the camera picker,
and three icons), below it one caption line and one row of controls. There is no
side panel. The source of truth for every value is `web/tokens.css`; this file
says why the values are what they are. The full study, with the rejected
directions A and C, is `docs/design/redesign-v1/proview-redesign-v1.pdf`.

## Principles

1. **The picture wins.** It is the largest and the brightest element on every
   screen: 72 % of a 1440×900 window on Live. Chrome that does not change what
   you see moves out of the way.
2. **Colour means a state.** Neutral graphite everywhere, and one meaning per
   colour: red only while recording, yellow only for data that is not fresh,
   green only for online or supported, blue only for the keyboard focus. The
   primary button is ink, not an accent.
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
| `--bg` | `#0f1112` | `#dfe0e2` | page, top bar |
| `--bg-2` | `#1b1d1f` | `#f0f1f2` | inputs, menus, raised surfaces |
| `--bg-3` | `#242629` | `#d3d5d7` | hover, selected menu row, skeleton |
| `--line` | `#37393c` | `#b0b2b5` | input borders, table rules |
| `--line-soft` | `#2d2f32` | `#bdbec1` | row separators, top bar |
| `--ink` | `#f1f2f4` | `#1c1e21` | main text, primary button fill, selection |
| `--ink-2` | `#cacccf` | `#424447` | control labels, secondary text |
| `--ink-3` | `#a6a9ae` | `#5e6166` | clock, hints, meta |
| `--stage` | `#030405` | `#030405` | behind the video, both themes |
| `--stage-ink` / `--stage-ink-2` | `#cfd1d3` / `#96989c` | `#d6d8da` / `#9c9fa2` | text drawn on the stage |
| `--live` | `#fc665f` | `#b10d1a` | recording, the only accent |
| `--ok` | `#6cce8a` | `#176b39` | online dot, "supported" |
| `--warn` | `#edb95d` | `#875a04` | frozen, stale, offline, a failed tile |
| `--bad` | `#fb7172` | `#b9232d` | form errors, destructive actions |
| `--focus` | `#7dc3ff` | `#0072cb` | the focus ring, nothing else |

Derived tokens are alpha over a listed colour, never a new hue: `--live-soft`
(live at 16 % / 10 %), `--stage-matte` (stage at 90 %, the move pad and the
full-screen line), `--stage-line` and `--stage-hover` (white at 14 % / 8 % on
the stage). No rule uses `color-mix()` or `oklch()`, so the colours do not
depend on what the WebView parses.

### Contrast

`python3 tools/contrast.py` parses `web/tokens.css`, composites every alpha
token over its real background, and exits non-zero when a pair falls under its
floor; CI runs it in the `check` job. Floors: 4.5:1 for text (11 px text
included), 3:1 for dots and the focus ring, 1.6:1 for `--line` and 1.4:1 for
`--line-soft` (a floor this project sets for separators; input edges do not
rely on the line alone because inputs sit on `--bg-2`). The tightest pairs it
reports, all passing:

| Pair | Dark | Light |
|---|---|---|
| ink-3 on page | 8.03 | 4.71 |
| warn on page | 10.55 | 4.55 |
| live on live-soft (recording control) | 5.29 | 4.56 |
| bad on page | 6.95 | 4.76 |
| stage-ink-2 on the move pad over a white frame | 5.84 | 6.34 |
| focus ring on page / on stage | 10.03 / 10.87 | 3.72 / 4.17 |

52 of 52 pairs pass.

## Type

IBM Plex Sans and IBM Plex Mono, bundled as five latin-subset woff2 files in
`web/fonts/` with `OFL.txt` (SIL OFL 1.1, © IBM Corp.), served by the server at
`/fonts/*` and so inside the desktop app too. `system-ui` would mean Segoe, SF
or Cantarell depending on the WebView; bundling keeps one face on every OS and
works offline.

Fixed px scale: 30 display / 21 heading / 17 title / 15 UI / 14 body / 13 small /
11 mono data. No tracked capitals; the only capitals are the wordmark. Mono only
for machine data: clocks, ids, addresses, error codes.

## Space, shape, elevation

Spacing 4 · 6 · 8 · 12 · 16 · 20 · 24 · 32 · 40 · 56. Tight inside a control
(6–8), loose between groups (16–24); the page edge is 20 px and the stage runs
to it. Radius 4 (code chips), 6 (buttons, inputs, tiles), 10 (stage, menus);
pill only for switches and table status. Three layers: the flat page, `--bg-2`
for inputs and menus, and a shadow only on what floats (menus). No cards.

Fixed sizes: top bar 56, control row 64, control 36 (44 under
`pointer: coarse`, through `--hit`), menu row 48, pad key 40.

## Motion

120 ms for hover and press, 180 ms for a menu entering (opacity and 4 px),
`cubic-bezier(0.22, 1, 0.36, 1)`. Motion only marks a state change: no page
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

Menus use the Popover API when `HTMLElement.prototype.showPopover` exists
(light dismiss and top layer from the platform) and a `hidden`-toggle with an
outside-pointer handler otherwise; `<html data-popover>` records which path
ran. Both paths share the same roving focus (arrows wrap, Home/End, Escape and
Tab close and return focus). `tests/web/ui.test.mjs` runs the menu tests on both
paths. WebKitGTK 2.54 (Fedora 44) reports `showPopover`; the fallback was also
rendered there with the API deleted.

## Focus and targets

A 2 px `--focus` ring with a 2 px gap, on `:focus-visible` only. Every control
is at least 36×36 (`--hit`), 44 under `pointer: coarse`. A "Skip to controls"
link is the first stop on Live.

## Not used, on purpose

Hero-centred sign-in, cards with shadows, emoji icons (the icons are one inline
SVG sprite, 20 px, 1.5 stroke), gradients, tracked capital eyebrows, a coloured
left border as accent, blur, radius above 10 px, cream backgrounds, entrance
animation, empty-state illustrations, text over the video outside full screen,
numbers that were not measured.
