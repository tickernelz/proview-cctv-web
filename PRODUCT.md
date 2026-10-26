# Product

## Register

product

## Users

One person, at a desk: the owner of a PROVIEW/XMEye camera or NVR, on a Windows,
macOS or Linux machine. They already run the phone app; they want the camera on a
larger screen while working, at night, or while something is happening. They are
not a security operator with a wall of monitors — there is one camera, or a
handful, and they check it in bursts.

The job: open the app, see the live picture quickly, glance at what happened
earlier, and close it. It must feel like an appliance, not like a control room
that needs study.

## Product Purpose

Put the vendor's camera stream on a desktop with less friction than the official
phone app, and do it without sending credentials anywhere except the vendor and
the local server. Success is: click → picture in about a second; the window is
still responsive with four tiles; nothing in the interface lies about what it
cannot do yet.

The same engine ships two ways: a self-hosted server binary for people who want
to reach it from another machine, and a native desktop app for people who want
nothing listening on their network.

## Brand Personality

Calm appliance. Quiet, precise, unshowy. The video is the only bright thing on
screen; the chrome recedes. Voice is plain and technical — it says "the device
did not answer", never "oops".

## Anti-references

- Dashboard-template look: gradient KPI cards, stock illustrations, big numbers
  with no meaning.
- Cluttered NVR software: a dozen panels and labels competing at once.
- Toy-like interfaces: bouncy animation, saturated colour everywhere, playful
  iconography in a security tool.

## Design Principles

1. **The picture wins.** Every pixel of chrome is paid for by video size; if a
   control does not change what you see, it belongs behind a menu.
2. **State, never decoration.** Colour and motion exist to say live, offline,
   loading, failed. Nothing animates to look alive.
3. **Say what is true.** An unwired feature shows a sentence explaining what is
   missing; a failed request shows the failure. No placeholder that pretends.
4. **One vocabulary.** The same button shape, the same row, the same status dot
   everywhere; a second visual language is a bug.
5. **Earned familiarity.** Standard desktop affordances — a device list, a
   player, a date picker — done exactly as expected, not reinvented.

## Accessibility & Inclusion

WCAG 2.1 AA. Body text ≥ 4.5:1 (measured, see `DESIGN.md`), visible
`focus-visible` ring on every control, status conveyed by text as well as colour
(the online dot carries an accessible name), `prefers-reduced-motion` disables
spinner and shimmer, keyboard-complete: login, device selection, player controls,
and fullscreen without a mouse.
