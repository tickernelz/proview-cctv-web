# Desktop notifications and the tray

The desktop build raises the alarm itself. One native system notification per new
alarm, posted by the desktop process rather than by the page, so a window that is
behind another window, minimized, or closed to the tray still gets it. There is no
push service behind this and nothing to register: the process asks the alarm centre
the same question the Alarm messages sheet asks, on a timer, while the app runs.

## What the app does

| Piece | Behaviour | Where |
|---|---|---|
| Alarm check | every *Alarm check interval* seconds (60 by default, 15 to 3600) the process reads the newest alarms across the account's devices; the device list itself is cached for ten minutes | `src-tauri/src/watcher.rs`, `crates/server/src/desktop.rs` |
| One notification per alarm | keyed by device serial, channel and alarm id, because an alarm id is a local timestamp with one-second resolution and two cameras or two channels can produce the same one | `crates/server/src/desktop.rs` (`DesktopAlarm::key`), `src-tauri/src/desktop_state.rs` |
| Remembering what was announced | the keys already announced, with their alarm times, live in `desktop-state.json` in the data directory; entries are dropped by time — anything older than the start of the requested window — never by count, so an id cannot fall off a list and come back. A file written by an older build, with plain id strings, does not parse and is treated as no state at all: one quiet baseline pass, nothing announced twice | same |
| First run | the first pass on an install that has no saved state records the last hour and announces nothing | same |
| A run after a gap | a run means the process starting with notifications on, or the switch turning on. The first check of a run treats alarms from before the run — older than the check interval plus the overlap — as history: they are recorded, not toasted, and one summary notification says how many arrived while PROVIEW was closed | `src-tauri/src/watcher.rs` |
| Notification text | title `<device> · <event>`, body the alarm time | `src-tauri/src/notify.rs` |
| Notification click | focuses the window and opens the Alarm messages sheet with that alarm marked, where the platform reports the click | `src-tauri/src/notify.rs`, `web/alarms.js` |
| Reaching the page | the desktop hands an event to the page with `WebviewWindow::eval`, which dispatches a DOM `CustomEvent` (`desktop-alarm`, `desktop-live`); the page listens with `addEventListener`. Tauri's own event channel is not used: the window is loaded from `http://127.0.0.1:<port>/`, which Tauri treats as a remote origin, and a capability only covers the local one, so `event.listen` is refused there. The page needs no Tauri permission for any of this, and the app ships no capability file | `src-tauri/src/notify.rs`, `web/alarms.js`, `web/app.js` |
| Session | the check reuses the session the window signed in with; the shell and the server share one session store, so the desktop never asks for a password of its own and never stores one | `crates/server/src/app.rs` (`serve_shared`) |
| Session lifetime | that shared session expires `PROVIEW_SESSION_TTL` seconds after sign-in (3600 by default) and reading it does not extend it, so an app left in the tray longer than that cannot reach the alarm centre until the window signs in again; the check says so instead of going quiet, see below | `crates/core/src/config.rs`, `crates/core/src/session.rs` |
| A check that fails | once a check has succeeded in this run, the first later check whose error says the session is gone posts one *Alarm notifications paused — open PROVIEW and sign in* notification and logs at warn; the rest of that span stays quiet, and a successful check re-arms it. Before any success the check announces nothing — the session store is empty at every launch, so a pause then would be a false alarm — but a hide to the tray does, see *Hidden before any success*. A cloud or network error logs at warn and posts nothing, because the session is still there | `src-tauri/src/watcher.rs` |
| A revoked vendor token | the vendor token is a 30-day JWT, so a token the vendor revokes early is the only way to reach this before the session store's own hour runs out: `msdlistCode` answers `-4004`, which `device_list` reports as `CloudError::Expired` and `since` as `AlarmReadError::SignedOut`, so it takes the pause path above — one notification and one warn line — instead of the quiet cloud-error log | `crates/core/src/cloud/envelope.rs`, `crates/server/src/desktop_alarms.rs` |
| Recovering from an outage | when two or more checks in a row failed, the recovering check summarizes everything it finds — one *N alarms came in while PROVIEW was not watching* notification and no per-alarm toast — instead of replaying a backlog one notification at a time. A single missed check still toasts normally. The rule counts checks, never wall-clock seconds, so timer jitter cannot flip it | `src-tauri/src/watcher.rs` |
| Another account | the remembered window belongs to the account that signed in; when a different account signs in, the window and the device cache are dropped and the run starts over, so one account's alarms can never silence another's | `src-tauri/src/desktop_state.rs`, `crates/server/src/desktop_alarms.rs` |
| Tray | tray icon with **Open**, **Live** and **Quit**; a left click on the icon focuses the window | `src-tauri/src/tray.rs` |
| One instance | a second launch — the Start menu shortcut, the Dock, a pinned icon — focuses the window of the running process instead of starting a second server, watcher and session store | `src-tauri/src/main.rs` (`tauri-plugin-single-instance`) |
| Window close | with *Keep running when the window closes* on (the default) the window hides and the app stays in the tray; the first time it happens, one notification says the app is still running — or the pause notice instead when the run never signed in, and then the still-running hint waits for the next hide — and the hint is recorded so it never repeats. If the system drops notifications, the tray icon is the only signal that the app is still there | `src-tauri/src/main.rs`, `src-tauri/src/tray.rs` |
| Window minimize | with *Hide to tray when minimized* on (off by default) minimizing hides the window instead of leaving it in the taskbar, with the same pause notice as the close path when the run never signed in | same |
| Hidden before any success | when notifications are on and no check has succeeded in this run, the first hide to the tray — close or minimize — posts one *Alarm notifications paused — open PROVIEW and sign in* notification and the rest of that run stays quiet; a run that has checked successfully has nothing to announce, because the check path already owns the pause there | `src-tauri/src/main.rs`, `src-tauri/src/shell.rs`, `src-tauri/src/desktop_state.rs` |
| macOS Dock click | `RunEvent::Reopen` brings the hidden window back | `src-tauri/src/main.rs` |
| No tray available | the app checks for the AppIndicator library before it builds the tray and treats a panic in the tray backend as a failure; either way both window behaviours stay off, so a window can never be hidden with no way back, and the log says `no system tray; the window closes normally` | `src-tauri/src/tray.rs` |

Nothing runs after the app quits. The check lives in the desktop process, so quitting
it — Quit in the tray, the window with *Keep running when the window closes* off, or
the end of the session — ends the notifications with it. A closed tab in the browser
build gets nothing; that is what the browser Notification API path is for and it needs
the tab open.

## When the session ends

The desktop has no password of its own: it borrows the session the window created, and
that session has a fixed life of `PROVIEW_SESSION_TTL` seconds — an hour by default.
Nothing renews it on read. On hour two of an app sitting in the tray, the alarm centre
answers "not signed in", and the process now says that out loud: one notification, one
warn line, and silence until the window signs in again. When it does, the alarms that
arrived during the gap arrive as one summary, not as a burst.

Making the desktop outlive that limit — keeping the vendor token past the page session,
or letting the shell restore a remembered account by itself — changes what a sign-out
means, so it stays a product decision rather than a detail of this feature.

## The request window

The captured alarm reply carries both clocks for one event: `AlarmTime
"2026-10-27 20:33:59"` next to `UtcTime 1793108041`, which is 13:34:01 UTC — a +7
offset, the account's own local time. `UtcTime` is a real epoch and is what the app
remembers; which clock the centre compares `StartTime` and `EndTime` against is not
proven, so the check asks for a window padded by fourteen hours on each side, which
covers every real timezone offset from UTC−12 to UTC+14. The keys dedupe the overlap,
and the padded window stays at 28 hours — one page of sixty entries per device at the
measured 52 alarms a day. An account far busier than that could outgrow the page, since
the offset field is not used yet.

The remembered watermark never runs ahead of the check itself, so one device with a
clock hours in the future cannot push the window past every other device's alarms.

## Settings

Settings → Configuration → Desktop. All four are stored in `settings.json` with every
other key, take effect without a restart, and are read from that file only — none of
them is exported into the environment. The group is hidden when the same page is served
by a self-hosted server, where nothing reads those keys.

| Key | Label | Default | Meaning |
|---|---|---|---|
| `PROVIEW_DESKTOP_ALARM_NOTIFY` | Alarm notifications | off | Post a system notification for each new alarm |
| `PROVIEW_DESKTOP_ALARM_INTERVAL` | Alarm check interval | 60 | Seconds between checks, 15 to 3600 |
| `PROVIEW_DESKTOP_CLOSE_TO_TRAY` | Keep running when the window closes | on | Closing the window hides it instead of quitting |
| `PROVIEW_DESKTOP_MINIMIZE_TO_TRAY` | Hide to tray when minimized | off | Minimizing hides the window |

Turning *Alarm notifications* on is the moment the app asks the system for permission.
On desktop the operating system owns that decision instead of an in-app prompt: Windows
and macOS decide per app in their notification settings, and the notification plugin
answers "granted" there, so the switch only records the user's own choice. If
notifications stay off at the system level, the app posts them and the system drops
them; the log line still says `notification posted`.

## Clicking a notification

A click is reported back to the app where the platform reports it: while the toast is
on screen on Windows, and as the notification server reports it on Linux and macOS.
When the click arrives, the window is focused and the alarm opens. When the platform
drops the click — a Windows toast already in the Action Center, a notification centre
that does not forward activation — the alarm is still in the sheet under the bell, and
the tray keeps the window one click away.

## Linux tray dependency

The tray is an AppIndicator. Building it needs `libayatana-appindicator3-dev` and
running it needs `libayatana-appindicator3.so.1`; both desktop workflows install the
build package with the other GTK/WebKit ones, and no server job does. Without the
runtime library the app starts normally: the tray reports that it could not be built,
the log says `no system tray; the window closes normally`, and both tray behaviours
stay off.

## What one check costs

The shape is fixed: one signed device-list call, cached for ten minutes, then one alarm
query per device per check, run one after another, each asking for a 28-hour window
capped at sixty entries. `desktop-state.json` is rewritten only when a check changed
something, so an idle window does not touch the disk every minute.

Measured here, in the desktop container under Xvfb, 240 s per run, reading the process's
own CPU ticks from `/proc/<pid>/stat`:

| Alarm notifications | CPU over 240 s | Checks due in the run |
|---|---|---|
| off | 8 ticks, 0.08 s | 0 |
| on, interval 15 s | 7 ticks, 0.07 s | 16 |

Both sit at roughly a thirtieth of a percent of one core and the difference is inside
run-to-run noise. That number covers the timer, the settings read and the wake-up only.
It does **not** include a vendor round trip: this workstation has no cloud account
session, since signing in needs the account password plus a verification code, so every
check here stops at "not signed in" before any network call. No request count, wall time
or response size per check against the real alarm centre is claimed; the shape above is
what a signed-in machine will do, one query per device per check, and at the measured 52
alarms a day that is one page of sixty entries per device per check.

## Checking it on a headless box

`proview-desktop --notify-probe` posts one fixed notification from a started shell and
exits three seconds later. `proview-desktop --event-probe` waits for the window, adds a
listener to `desktop-alarm`, dispatches an event through the same Rust path a
notification click uses, and writes what the page saw into the window title before it
exits. Both write `desktop.log` in the data directory and to stderr.
