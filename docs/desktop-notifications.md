# Desktop notifications and the tray

The desktop build raises the alarm itself. One native system notification per new
alarm, posted by the desktop process rather than by the page, so a window that is
behind another window, minimized, or closed to the tray still gets it. There is no
push service behind this and nothing to register: the process asks the alarm centre
the same question the Alarm messages sheet asks, on a timer, while the app runs.

## What the app does

| Piece | Behaviour | Where |
|---|---|---|
| Alarm check | every *Alarm check interval* seconds (60 by default, 15 to 3600) the process reads the newest alarms across the account's devices | `src-tauri/src/watcher.rs`, `crates/server/src/desktop.rs` |
| One notification per alarm | keyed by the alarm id; the ids already seen, and the newest alarm time, live in `desktop-state.json` in the data directory, so a restart does not replay what was on screen before it | `src-tauri/src/desktop_state.rs` |
| First run | the first pass records the last hour without posting anything; only what arrives afterwards is announced | same |
| Notification text | title `<device> · <event>`, body the alarm time | `src-tauri/src/notify.rs` |
| Notification click | focuses the window and opens the Alarm messages sheet with that alarm marked | `src-tauri/src/notify.rs`, `web/alarms.js` |
| Session | the check reuses the session the window signed in with; the shell and the server share one session store, so the desktop never asks for a password of its own and never stores one | `crates/server/src/app.rs` (`serve_shared`) |
| Tray | tray icon with **Open**, **Live** and **Quit**; a left click on the icon focuses the window | `src-tauri/src/tray.rs` |
| Window close | with *Keep running when the window closes* on (the default) the window hides and the app stays in the tray; the first time it happens, one notification says the app is still running, and the hint is recorded so it never repeats | `src-tauri/src/main.rs`, `src-tauri/src/tray.rs` |
| Window minimize | with *Hide to tray when minimized* on (off by default) minimizing hides the window instead of leaving it in the taskbar | same |
| No tray available | if the tray cannot be installed, both window behaviours stay off, so a window can never be hidden with no way back; the log says `no system tray; the window closes normally` | `src-tauri/src/main.rs` |

Nothing runs after the app quits. The check lives in the desktop process, so quitting
it — Quit in the tray, the window with *Keep running when the window closes* off, or
the end of the session — ends the notifications with it. A closed tab in the browser
build gets nothing; that is what the browser Notification API path is for and it needs
the tab open.

## The request window

The captured alarm reply carries both clocks for one event: `AlarmTime
"2026-10-27 20:33:59"` next to `UtcTime 1793108041`, which is 13:34:01 UTC — a +7
offset, the account's own local time. `UtcTime` is a real epoch and is what the app
remembers as "the newest alarm seen"; which clock the centre compares `StartTime` and
`EndTime` against is not proven, so the check asks for a window padded by twelve
hours on each side. A timezone convention difference therefore cannot drop an alarm,
the ids dedupe the overlap, and the padded window stays at 24 hours — one page of
sixty entries at the measured 52 alarms a day. An account far busier than that could
outgrow the page, since the offset field is not used yet.

## Settings

Settings → Configuration → Desktop. All four are stored in `settings.json` with every
other key, take effect without a restart, and are read from that file only — none of
them is exported into the environment.

| Key | Label | Default | Meaning |
|---|---|---|---|
| `PROVIEW_DESKTOP_ALARM_NOTIFY` | Alarm notifications | off | Post a system notification for each new alarm |
| `PROVIEW_DESKTOP_ALARM_INTERVAL` | Alarm check interval | 60 | Seconds between checks, 15 to 3600 |
| `PROVIEW_DESKTOP_CLOSE_TO_TRAY` | Keep running when the window closes | on | Closing the window hides it instead of quitting |
| `PROVIEW_DESKTOP_MINIMIZE_TO_TRAY` | Hide to tray when minimized | off | Minimizing hides the window |

Turning *Alarm notifications* on is the moment the app asks the system for
permission. On desktop the operating system owns that decision instead of an in-app
prompt: Windows and macOS decide per app in their notification settings, and the
notification plugin answers "granted" there, so the switch only records the user's
own choice. If notifications stay off at the system level, the app posts them and
the system drops them; the log line still says `notification posted`.

## Clicking a notification

A click is reported back to the app where the platform reports it: while the toast is
on screen on Windows, and as the notification server reports it on Linux and macOS.
When the click arrives, the window is focused and the alarm opens. When the platform
drops the click — a Windows toast already in the Action Center, a notification centre
that does not forward activation — the alarm is still in the sheet under the bell, and
the tray keeps the window one click away.

## Linux tray dependency

The tray is an AppIndicator. Building it needs `libayatana-appindicator3-dev` and
running it needs `libayatana-appindicator3-1`; both desktop workflows install the
build package with the other GTK/WebKit ones, and no server job does. Without the
runtime library the app still starts: the tray fails, the log says
`no system tray; the window closes normally`, and both tray behaviours stay off.

## What is measured here and what is not

Measured on this workstation (no graphical desktop, so everything runs headless):

- `cargo test -p proview-desktop`: the dedupe, the restart behaviour, the baseline
  pass, the bounded seen list, and the four settings defaults.
- `cargo test -p proview-core -p proview-server`: the shared session store's
  "newest live session", the desktop settings keys with their defaults and bounds, the
  alarm-to-notification mapping, and the padded request window.
- Under Xvfb: the app starts, installs the tray, and posts a notification without
  panicking (`notification posted` in `desktop.log`).

Not measured, and not claimed:

- A notification seen on a real Windows or macOS desktop. This box has no display
  server for either, so the honest statement is "posted by the plugin on that build and
  built by CI", not "seen".
- A mouse click on a desktop notification, for the same reason. The handler is wired
  and the click path is exercised through the plugin's action callback, but no click
  has been delivered by a real notification server here.
- Delivery to a live notification daemon on Linux, where the plugin posts over D-Bus.
