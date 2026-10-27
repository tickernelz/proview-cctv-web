# Design

Visual system for the PROVIEW CCTV web app. Source of truth is
`web/tokens.css`; this file explains the intent behind it.

## Theme

Night watch room: one red LED in the dark, everything else quiet. The video is
the brightest thing on screen, so the interface around it stays low-contrast,
cool-neutral and flat. Dark is the default because monitoring happens in the
evening; light is a first-class second theme, not an inversion hack.

## Color

All colours are OKLCH tokens. Brand anchor: warm crimson, taken as the
"live / recording" signal — the one place colour is allowed to shout.

| Role | Dark | Light | Use |
|---|---|---|---|
| `--bg` | `oklch(0.16 0.008 265)` | `oklch(0.965 0.003 265)` | page |
| `--surface` | `oklch(0.21 0.01 265)` | `oklch(1 0 0)` | bars, rows |
| `--surface-2/3` | `0.26` / `0.31` | `0.945` / `0.91` | raised controls, hover |
| `--line` / `--line-soft` | `0.34` / `0.27` | `0.88` / `0.93` | borders |
| `--stage` / `--stage-matte` | `0.11` / `0.20` | same | video, player matte |
| `--ink` / `--ink-dim` / `--muted` | `0.96` / `0.84` / `0.74` | `0.22` / `0.34` / `0.50` | text |
| `--primary` | `oklch(0.62 0.19 27)` | `oklch(0.52 0.19 27)` | primary action, live/selected |
| `--success` `--warning` `--danger` `--info` | hue 155 / 78 / 22 / 245 | darker variants | device online, warnings, errors, focus |

Contrast is verified numerically with `tools/contrast.py`, which parses
`web/tokens.css` itself (OKLCH → sRGB → relative luminance) and exits non-zero
when a pair falls under its floor: 4.5:1 for text, 3:1 for graphics. Measured,
not eyeballed:

| Pair | Dark | Light |
|---|---|---|
| ink on bg | 17.28:1 | 15.65:1 |
| ink on surface | 15.78:1 | 17.32:1 |
| ink-dim on surface | 10.85:1 | 11.76:1 |
| muted on bg | 8.41:1 | 5.42:1 |
| muted on surface | 7.68:1 | 6.00:1 |
| muted on surface-2 | 6.74:1 | 5.11:1 |
| muted on primary-soft (selected device row) | 7.45:1 | 4.63:1 |
| primary-ink on primary | 4.81:1 | 5.90:1 |
| success on surface | 7.61:1 | 5.16:1 |
| danger on surface | 5.21:1 | 5.83:1 |
| stage-ink on stage | 15.18:1 | 15.18:1 |
| stage-danger on stage-matte | 7.30:1 | 7.30:1 |
| focus-ring on bg | 9.74:1 | 5.27:1 |
| on-scrim over a white frame | 14.48:1 | 14.48:1 |
| on-scrim over a mid frame | 17.13:1 | 17.13:1 |

Every text pair clears 4.5:1. Two pairs only clear the 3:1 graphics floor and
are never used as text: `--primary` as an icon fill on `--surface` in dark
(4.44:1) and `--warning` as a status dot or a notice border in light (4.02:1).
`--on-scrim` is measured against the worst case, a white video frame behind an
88% scrim, because the picture under the overlay is uncontrolled.

The stage is dark in both themes, so anything drawn on it uses stage tokens that
do not flip with the theme: `--stage-ink` and `--stage-danger`. A theme-flipping
`--muted` measured 3.01:1 on the stage in the light theme, which is why those
tokens exist.

The accent is never used for decoration; if a control is not primary, selected,
or in error, it is neutral.

## Typography

One family: `ui-sans-serif, system-ui, …`. Product UI, so a fixed rem scale
(0.75 / 0.8125 / 0.9375 / 1.0625 / 1.25 / 1.5), not fluid clamp headings.
Mono (`ui-monospace`) is reserved for machine data: device ids, model strings,
timestamps. Display headings use `letter-spacing: -0.02em`, well inside the
-0.04em floor.

## Layout

- One content column inside a 1440px page, generous vertical rhythm from a
  4px-based space scale (4/8/12/16/24/32/48).
- The stage is the anchor: black, rounded 14px, sized from the picture's own
  aspect — or the lens crop's when one lens is selected — and it never shrinks
  for chrome.
- Rows over cards. A device is a row with a status dot, a name, mono meta and
  one action — not a card, never a nested card.
- Responsive is structural only: the row collapses to two columns under 720px.

## Components

Every interactive element implements default, hover, focus-visible, active,
disabled and loading. The sign-in button demonstrates the loading contract:
`aria-busy` swaps the label for a spinner without changing its width.

- Status dots carry an accessible name (`role="img"`, "Online"/"Offline"), not
  colour alone.
- Empty states teach: "No devices on this account yet. Add the camera in the
  phone app first, then refresh here."
- Errors are inline text tied to `role="alert"`, never a modal. The stage has
  its own error state so a failed stream does not blank the page.

## Surfaces

**Login.** One panel, two modes as tabs: cloud account (default) and this
network. The local tab posts the camera username and password to this server,
which signs in to the camera address from its own environment; the browser keeps
no copy, only the server's opaque session cookie. The tab stays disabled until
`GET /api/config/mode` reports the local provider, and it says where to switch.
A separate probe button reads the camera's DVRIP login and RTSP description so a
wrong address fails on the login screen instead of after it. Both modes end in
the same place: the device list.

**Shell.** A 260px device rail and a content column. The rail scrolls on its
own; the content column never shrinks below its stage. Under 860px the rail
folds above the content and the nav collapses to icons.

**Live.** Lens picker, channel picker, stage, control bar, lens caption and one
disclosure. The stage spans the content column and its height follows the
picture's aspect, capped at 66% of the viewport, so the lens caption stays above
the fold at 900px. The picture is centred inside the stage at its exact aspect:
a 16:9 lens fills the width, and the stacked dual-lens frame is taller than it
is wide, so it keeps its own proportions with the player surface around it. The
OSD sits inside the picture box, not at the frame's corners. The stage is the
fullscreen target.

**Stage states.** Four, each named on screen: `loading` ("Requesting stream…"
with a spinner), `playing`, `stalled` and `error`. `playing` begins only after the
browser has decoded a frame, so a tile never claims to play while it is blank; a
socket that opens and sends nothing decodable becomes `error` after six seconds.
A picture that stops advancing while the socket stays open becomes `stalled`: the
last frame stays on screen, a scrim caption says how long it has been frozen and
that the clock on it is not the time of what is shown, and snapshot stays enabled
because the frame is real. An open connection is never treated as proof of a
moving picture.

**Lens views.** Combined shows the frame the camera sent. A single lens crops
one region of that same frame in the browser — no second connection, no second
decode. Crops come from the lens contract when the server publishes one; until
it does, the geometry of the frame decides the split and the caption says so.
The caption is not decoration: it is the only place that states whether the
layout was reported or inferred.

**Grid.** One tile per channel of the selected device, `auto-fit` at 320px
minimum: two channels sit side by side from 1000px, four land as a 2×2 at
1280px. Every tile shows the whole frame, not
the lens crop chosen in the live view: the grid is an overview, and a tile that
silently cropped half the picture would hide a lens. Each tile is its own stream
and its own failure: a channel with no recording says so inside that tile while
the others keep playing. Tiles open the live view on their channel.

**Control panel.** Two groups with different honesty. In-browser controls act
immediately (mute, record, snapshot, flip, quality, expand). Device controls are
driven by the server's own capability report: the panel calls
`GET /api/device/controls` once per device, disables each button whose feature
the report marks `blocked`, and prints the device's evidence under "What the
device reports it can do". A button is enabled only when its feature is
`supported` for the lens it would move, so pan and tilt follow the motorised
lens and refuse the fixed one before a packet leaves. Enabled actions call their
real endpoint and echo the server's answer: `POST /api/device/controls/ptz`
reports the lens, backend and hold time it applied, presets are read from
`GET /api/device/controls/presets` and listed by name, and the white light
toggles through its current mode. Nothing in the panel pretends to have moved a
camera.

**Source badge.** A pill over the stage naming transport, quality, channel and
the source that answered — `cloud`, `local` or `fixture`, taken from the stream
ticket's own `source` field and from `GET /api/config/mode`. The ticket URL,
expiry, chosen mode and any fallback notice live in its title and accessible
name. It is the answer to "why am I looking at this picture", so it never
guesses: before a ticket exists it says `ticket pending`, and after a failure it
says `Stream failed`.

**Settings.** Appearance, connection, playback defaults, configuration and a
server check panel. Connection reads `GET /api/config/mode` and writes it back
with `POST /api/config/mode`; when the chosen mode cannot be served the server's
own fallback notice is shown verbatim, prefixed with "Fallback:". The checks call
`/api/health`, `/api/me`, `/api/device/rtsp` and `/api/stream` and print the
real status, including a refused camera address. Display preferences live in
`localStorage`; the page never claims to have changed server state it did not
change.

**Configuration.** One fieldset per group (Cloud, Local network, Server,
Streaming, Advanced), each field a label, its control and one line of hint — the
hint carries the example or the consequence, never a second label. A key read
once at startup says "needs restart" inside its label, so the marker travels
with the field instead of living in a legend two screens away. Saving sends only
what changed, and the apply row appears under the buttons only when a saved key
needed a restart: the desktop shows **Restart the app**, a server says to restart
the process, because a control that cannot work is worse than a sentence. Secret
fields render empty with "stored, type to replace" as the placeholder; nothing
reads a stored secret back into the page.

**Settings before sign-in.** The same screen opens from the sign-in form, where
no session exists yet. In that mode the shell drops the device rail and the Live
and Grid links, the account chip and sign-out are absent, and `Back to sign in`
takes the top-right place the account chip normally holds. The screen has to be
complete without a session: this is the first thing a new install sees.

**Lens roles.** The moving lens pans and tilts from the app; the fixed lens is
aimed by turning the housing by hand. The picker labels each lens by role —
"Top lens (fixed)", "Bottom lens (moving)" — only when the lens contract reports
`ptz` for it. Before that it labels them by position, and the disclosure says
which half is which and why a fixed lens that refuses to pan is hardware, not a
fault.

## Motion

150–250 ms, `cubic-bezier(0.22, 1, 0.36, 1)`. Motion only for state: spinner,
skeleton shimmer, hover/active feedback. `prefers-reduced-motion` collapses the
duration tokens to 0 and stops the shimmer and spinner.

## Z-index

Semantic scale only: `--z-sticky` 100, `--z-dropdown` 200, `--z-overlay` 300,
`--z-modal` 400, `--z-toast` 500. No arbitrary 999s.

## Anti-references

- Dashboard-template gradients, stock illustration, hero KPI tiles.
- Dark mode as pure `#000` with `#fff` text and no neutral ramp.
- Camera grids where every tile shouts; only the active tile may take the
  accent.
