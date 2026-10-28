# Redesign v2: two directions

ZHA-4181, 2026-10-28. Zhafron asked for a UI that is responsive, fluent, tidy
and easy to use, and handed every design choice to the engineer (16:17Z).
Both options below were judged against the same gates (DESIGN.md), on the
v0.3.5 defect list.

## What was wrong in v0.3.5

| # | Defect | Where |
|---|---|---|
| 1 | Alarm bell is an emoji (U+1F514), the only colour glyph on the page | `alarms.js` |
| 2 | Playback and Cloud recordings share the clock icon; Devices has none | `playback.js`, `recordings.js`, `device-admin.js` |
| 3 | Hand-drawn icon sprite: 27 symbols, mixed stroke weights | `index.html` |
| 4 | Alarm sheet, playback and cloud recordings inject their own CSS with hex colours and two tokens that do not exist (`--bg-1`, `--dim`): they ignore the light theme | three scripts |
| 5 | Pressed, selected and checked look like hover (ink on ink) | `style.css` |
| 6 | Text at 10-11 px in the badge, pad notes and alarm footer | several |
| 7 | 6 px and 10 px steps break the 4 px rhythm | `style.css` |
| 8 | Sign-in, settings and the top bar scroll sideways at 320-390 px; auto grid uses 3 columns at every width | measured by the responsive gate |

## Option 1: Graphite + one accent (chosen)

Keep the neutral graphite page so the camera picture stays the brightest thing
on screen, and give interaction one colour of its own: blue for primary,
pressed, selected and checked, with a soft fill behind it. Red stays recording
and alarm count only, green online, yellow stale. One icon family (Lucide),
a 4 px grid, 12 px floor for text, 160/200 ms motion.

- Fixes all eight defects without moving a control, so every flow a user knows
  stays where it was.
- Cost: CSS +1.3 KB gzip; JS -1.3 KB (no injected style strings).
- Contrast: 76 of 76 pairs pass; the weakest is the light focus ring on the
  stage, 3.31:1 against a 3:1 floor.

## Option 2: Surface cards

Put the live picture, the controls and every sheet on raised cards with a 16 px
radius over a tinted page, accent-coloured headers, larger 48 px controls.

- More "app-like" in a screenshot, but cards add an edge and a shadow around the
  picture, which v1 measured as the element that must win; the control row grows
  by 8 px, which costs the picture height on 1280x800 and below.
- Tinted pages lower ink-3 contrast in the light theme (the hint text was the
  tightest pair already).
- Every screen moves, so muscle memory from v0.3.5 is lost for no gain in a task.

## Why option 1

The complaints were that things did not match (icons), were not smooth (state
changes, motion) and did not fit (narrow screens), not that the layout was
wrong. Option 1 answers each with a measured change and keeps the picture as
the largest, brightest element. Screens rendered from the real build are in
`e2e/tests/visual.spec.ts-snapshots/` and `e2e/tests/touch.spec.ts-snapshots/`;
`proview-redesign-v2.pdf` shows them side by side with the defect list.
