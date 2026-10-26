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
| `--ink` / `--ink-dim` / `--muted` | `0.96` / `0.84` / `0.74` | `0.22` / `0.34` / `0.50` | text |
| `--primary` | `oklch(0.62 0.19 27)` | `oklch(0.52 0.19 27)` | primary action, live/selected |
| `--success` `--warning` `--danger` `--info` | hue 155 / 78 / 22 / 245 | darker variants | device online, warnings, errors, focus |

Contrast is verified numerically with `tools/contrast.py` (OKLCH → sRGB →
relative luminance). Measured, not eyeballed:

| Pair | Dark | Light |
|---|---|---|
| ink on bg | 17.28:1 | 15.65:1 |
| ink on surface | 15.78:1 | 17.32:1 |
| ink-dim on surface | 10.85:1 | 11.76:1 |
| muted on bg | 8.41:1 | 5.42:1 |
| muted on surface | 7.68:1 | 6.00:1 |
| muted on surface-2 | 6.74:1 | 5.11:1 |
| primary on bg | 4.87:1 | 5.49:1 |
| success on surface | 7.61:1 | 5.16:1 |
| danger on surface | 5.21:1 | 5.83:1 |
| warning on surface | 8.71:1 | 4.02:1 |
| on-scrim over a white frame | 14.48:1 | 14.48:1 |
| on-scrim over a mid frame | 17.13:1 | 17.13:1 |

Every text pair used in the interface clears 4.5:1 except two non-text usages:
`--primary` as an icon fill in dark (4.44:1, above the 3:1 graphics floor) and
`--warning` as a dot in light (4.02:1, same floor). Both are graphics, never
body text. `--on-scrim` is measured against the worst case, a white video frame
behind an 88% scrim, because the picture under the overlay is uncontrolled.

The accent is never used for decoration; if a control is not primary, selected,
or in error, it is neutral.

## Typography

One family: `ui-sans-serif, system-ui, …`. Product UI, so a fixed rem scale
(0.75 / 0.8125 / 0.9375 / 1.0625 / 1.25 / 1.5), not fluid clamp headings.
Mono (`ui-monospace`) is reserved for machine data: device ids, model strings,
timestamps. Display headings use `letter-spacing: -0.02em`, well inside the
-0.04em floor.

## Layout

- Single column, max 1080px, generous vertical rhythm from a 4px-based space
  scale (4/8/12/16/24/32/48).
- The stage is the anchor: 16:9, black, rounded 14px, and it never shrinks for
  chrome.
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
network. The local tab asks the server to probe the camera it is configured
with; the form never carries camera credentials, because those live in the
server environment. Both modes end in the same place: the device list.

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

**Lens views.** Combined shows the frame the camera sent. A single lens crops
one region of that same frame in the browser — no second connection, no second
decode. Crops come from the lens contract when the server publishes one; until
it does, the geometry of the frame decides the split and the caption says so.
The caption is not decoration: it is the only place that states whether the
layout was reported or inferred.

**Grid.** One tile per channel of the selected device, `auto-fit` at 360px
minimum so four channels land as a 2×2. Every tile shows the whole frame, not
the lens crop chosen in the live view: the grid is an overview, and a tile that
silently cropped half the picture would hide a lens. Each tile is its own stream
and its own failure: a channel with no recording says so inside that tile while
the others keep playing. Tiles open the live view on their channel.

**Control panel.** Two groups with different honesty. In-browser controls act
immediately (mute, record, snapshot, flip, quality, expand). Device controls
require the server's control channel: the panel probes
`GET /api/device/capabilities` once per session and, when that endpoint answers
404, renders every device button disabled with the status it actually received.
Nothing in the panel pretends to have moved a camera.

**Source badge.** A pill over the stage naming transport, quality, channel and
the source that answered — `cloud`, `local` or `fixture`, taken from the stream
ticket's own `source` field and from `GET /api/config/mode`. The ticket URL,
expiry, chosen mode and any fallback notice live in its title and accessible
name. It is the answer to "why am I looking at this picture", so it never
guesses: before a ticket exists it says `ticket pending`, and after a failure it
says `Stream failed`.

**Settings.** Appearance, connection, playback defaults and a server check
panel. Connection reads `GET /api/config/mode` and writes it back with
`POST /api/config/mode`; when the chosen mode cannot be served the server's own
fallback notice is shown verbatim, prefixed with "Fallback:". The checks call
`/api/health`, `/api/me`, `/api/device/rtsp` and `/api/stream` and print the
real status, including a refused camera address. Display preferences live in
`localStorage`; the page never claims to have changed server state it did not
change.

**Lens roles.** The moving lens pans and tilts from the app; the fixed lens is
aimed by turning the housing by hand. The interface says this in the lens
disclosure and labels a lens by role only when the server reports the role.
Until then it labels them by position — "upper" and "lower" — and the disclosure
says why.

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
