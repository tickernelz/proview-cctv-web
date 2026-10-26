# Design

Visual system for the PROVIEW CCTV web app. Source of truth is
`static/tokens.css`; this file explains the intent behind it.

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
| `--bg` | `oklch(0.16 0.008 265)` | `oklch(0.985 0.002 265)` | page |
| `--surface` | `oklch(0.21 0.01 265)` | `oklch(1 0 0)` | bars, rows |
| `--surface-2/3` | `0.26` / `0.31` | `0.962` / `0.93` | raised controls, hover |
| `--line` / `--line-soft` | `0.34` / `0.27` | `0.88` / `0.93` | borders |
| `--ink` / `--ink-dim` / `--muted` | `0.96` / `0.84` / `0.74` | `0.22` / `0.34` / `0.50` | text |
| `--primary` | `oklch(0.62 0.19 27)` | `oklch(0.52 0.19 27)` | primary action, live/selected |
| `--success` `--warning` `--danger` `--info` | hue 155 / 78 / 22 / 245 | darker variants | device online, warnings, errors, focus |

Contrast is verified numerically with `tools/contrast.py` (OKLCH → sRGB →
relative luminance). Measured, not eyeballed:

| Pair | Dark | Light |
|---|---|---|
| ink on bg | 17.28:1 | 16.59:1 |
| muted on bg | 8.41:1 | 5.75:1 |
| primary on bg | 4.87:1 | 5.82:1 |

All three pass WCAG AA for body text (4.5:1). The accent is never used for
decoration; if a control is not primary, selected, or in error, it is neutral.

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
