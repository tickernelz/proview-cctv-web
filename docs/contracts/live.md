# Live contract — the stage that failed, named

The live page used to answer one 502 for every failure: a dead cloud login, a
relay that refused the session and a device that never sent a frame all read the
same. This document is the contract between the server and the live page for the
diagnostic surface that replaces that 502, and it names where each field comes
from.

Types and routes live in `crates/server/src/routes/live.rs`; the route
registration is one line each in `crates/server/src/routes.rs`.

## 1. Stage vocabulary

Eight stages, always reported in this order:

| Stage | The call that decides it |
|---|---|
| `cloud_login` | the session carries a cloud token at all; no token stops here |
| `device_list` | `EnvelopeClient::device_list` returns a device whose uuid matches |
| `status_cluster` | `StatusClient::query` answers for that serial and auth code |
| `rps_access` | `AccessClient::assign` on the access server the status reply names |
| `tunnel` | `rps::open` handshake, and the relay answer is `ErrorNum 200` |
| `dvrip_login` | `Connection::login` or `login_token` through the tunnel |
| `opmonitor_claim` | `Connection::claim_monitor` on the channel and stream |
| `first_frame` | an access unit for the session arrived |

The vocabulary is fixed; the walk reports a stage only when it makes that call.
Today `provider/cloud.rs` stops after the device login, so the walk stops there
too and `opmonitor_claim` and `first_frame` appear only once the provider sends
those calls. A stage is never reported for a call the provider does not make.

## 2. Report shape

One stage report is exactly:

```json
{ "stage": "tunnel", "ok": false, "elapsed_ms": 1490, "detail": "the relay refused the session: ErrorNum 403" }
```

`elapsed_ms` is measured with `Instant` around that one call, never derived
from a neighbouring stage. `detail` quotes the vendor or the transport; it is
never a rewritten sentence about the symptom.

An attempt carries the reports it actually produced:

```json
{
  "device": "fixture-dev-0001",
  "channel": 1,
  "at": 1761600000000,
  "stage": "status_cluster",
  "detail": "cloud status unknown: the status ports did not answer",
  "elapsed_ms": 118,
  "stages": [ { "stage": "cloud_login", "ok": true, "elapsed_ms": 0, "detail": "..." } ]
}
```

`stage` is the stage of a report that **failed**, and it is absent when every
report passed. A walk that proves all six stages the cloud path makes today
answers `stage: null` and leaves the field out of the JSON; the provider's own
message is then the only answer, because the provider is where the run stopped.
`detail` follows `stage`: it is absent when `stage` is.

`stages` holds only stages the walk executed, so its length is the evidence: a
two-row list means the walk stopped at the second row.

## 3. Routes

`GET /api/stream` keeps its 200 shape on success. On failure it answers with
the status the provider error already earned (502 for an upstream or transport
failure, 404 for an unknown device, 401 for a lost session) and this body:

```json
{
  "ok": false,
  "message": "upstream error: no cluster reported an access server",
  "stage": "status_cluster",
  "detail": "cloud status unknown: the status ports did not answer",
  "elapsed_ms": 118,
  "stages": [ "..." ]
}
```

`message` is the provider's own error and always present. `elapsed_ms` and
`stages` are present whenever a walk ran; `stage` and `detail` only when a
stage failed. A fixture or LAN provider has no cloud path to walk, so its
failure keeps the older `{ok, message}` body.

`GET /api/live/stats` (session required) returns the hub counters plus the last
attempt of **that session**:

```json
{
  "active_streams": 0,
  "streams": [ "..." ],
  "frames": 0,
  "bytes": 0,
  "gaps": 0,
  "attempts": [ "..." ]
}
```

`frames` and `bytes` are summed from the hub's `packets` and `bytes`;
`gaps` is the hub's `drops`. `attempts` is one entry per (device, channel)
this session tried, newest first, and is empty until one of its streams failed.
An attempt's owner is not serialised, so another session never sees the device
ids, the account or the vendor words of a session it does not own.

## 4. When the walk runs, and how often

The walk runs on a `GET /api/stream` failure only, and only when the provider is
the vendor cloud, the router resolved to the cloud source, and the error is not
a credentials, expiry, unknown-device, not-implemented or misconfiguration
error. Those five name their own cause and a walk would add vendor calls without
adding an answer.

A successful ticket drops the attempt for that (device, channel), so
`/api/live/stats` never shows a failure that a later attempt disproved.

The vendor relay allows one session per device, and the walk no longer depends on
the channel — it stops at the device login and never claims a monitor — so the
walk is single-flight **per device**: a second failure for any channel of that
device waits for the walk already running and then reuses its report instead of
opening a second relay session. The reused report is returned with the channel
the caller asked for; its ledger row stays per (device, channel) so
`/api/live/stats` still lists each channel that failed.

The window is 5 s (`WALK_COOLDOWN`); a reused report keeps its original `at`.
The whole walk shares one 8 s budget (`WALK_DEADLINE`): each stage takes what is
left of it, and a stage that starts with nothing left reports `the walk spent its
8000 ms budget before this stage`. A wedged vendor therefore cannot hold
`/api/stream` open past that budget.

## 5. The walk, and what it duplicates

`crates/core/src/provider/cloud.rs` walks these same calls inside `stream()`
but answers one string, so a per-stage duration cannot be recovered from it.
The walk in `routes/live.rs` calls the same public `proview_core` primitives —
`cloud::envelope`, `cloud::status`, `provider::rps`, `provider::p2p`,
`dvrip::client` — in the same order and with the same arguments, one stage at a
time, and times each. It stops where the provider stops: no monitor claim and no
media read, so it never asks the device to stream into a tunnel that closes.

When `provider/cloud.rs` grows a stage report of its own, the walk is deleted
and this route reads that report; until then this is the only place a per-stage
duration exists.

## 5b. What the walk costs

`/api/stream` on a stub cloud, three runs per cell, base `4d9ff07` against the
lane head, same stub and same account:

| Failure | base `4d9ff07` | lane head | Walk's share |
|---|---|---|---|
| cloud base refused | 1.38-1.77 ms | 1.05-2.05 ms | below the noise floor, one refused socket |
| device list without an rps id | 2.79-2.98 ms | 3.42-5.41 ms | 0.5-2.4 ms, one extra device list |
| stub ok, vendor status silent | 1.03-2.07 s | 2.05-4.02 s | 1.0-2.0 s, the walk repeats the status query |
| 4 parallel calls, one channel | 4 stub calls, 3.09 s wall | 5 stub calls, 4.13 s wall | exactly one walk for all four |

Both columns are the same six runs per binary; the lane column is the binary at
`ffdc902`, which is the commit this table ships with.

The third row is the worst case today and it is the vendor's status query, not
the walk's bookkeeping. The 8 s budget caps a wedged vendor below that.

The walk reads the status host from `STATUS_HOST`, the same constant the
provider uses; the host is a field on the walk so a test can point it at a stub,
and nothing in the running server changes it.

## 6. What the page shows

The live page prints `Failed at <stage> after <ms> ms: <detail>` where it used
to print "Camera offline", keeps the provider message in the tooltip line, lists
the per-stage trail, and offers **Copy the stage detail**, which copies that line
plus one line per stage with its own duration. When no stage failed the headline
stays the provider's sentence and the trail still lists the stages that passed.
