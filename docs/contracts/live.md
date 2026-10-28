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

`stage` is **the first stage the attempt did not complete**, in vocabulary
order. A run that proves every stage before `first_frame` therefore reports
`first_frame`: no frame arrived, which is what the page then says. A stage is
never reported as passed because a later one was reached; each report is the
outcome of its own call.

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

`message` is the provider's own error and stays for clients that only read it.
`stage`, `detail`, `elapsed_ms` and `stages` appear only when a walk ran: a
fixture or LAN provider has no cloud path to walk, and its failure keeps the
older `{ok, message}` body.

`GET /api/live/stats` (session required) returns the hub counters plus the last
attempt per device:

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
`gaps` is the hub's `drops`. `attempts` is one entry per device, newest
first, and is empty until a stream has failed.

## 4. When the walk runs, and how often

The walk runs on a `GET /api/stream` failure only, and only when the provider is
the vendor cloud, the router resolved to the cloud source, and the error is not
a credentials, expiry, unknown-device, not-implemented or misconfiguration
error. Those five name their own cause and a walk would add vendor calls without
adding an answer.

A successful ticket drops the device's attempt, so `/api/live/stats` never
shows a failure that a later attempt disproved.

The vendor relay allows one session per device, so a walk for the same device is
reused for 5 s (`WALK_COOLDOWN`) instead of opening a second relay session; the
reused report keeps its original `at`. Every stage is bounded at 8 s
(`STAGE_LIMIT`) and reports `no answer within 8000 ms` when the bound fires,
so a wedged vendor cannot hold `/api/stream` open.

## 5. The walk, and what it duplicates

`crates/core/src/provider/cloud.rs` walks these same calls inside `stream()`
but answers one string, so a per-stage duration cannot be recovered from it.
The walk in `routes/live.rs` therefore calls the same public primitives of
`proview_core` — `cloud::envelope`, `cloud::status`, `provider::rps`,
`provider::p2p`, `dvrip::client` — one stage at a time, and times each. It is
a diagnostic second pass over a path that has just failed, not a second live
path: it reads no media and closes the tunnel when it stops.

When `provider/cloud.rs` grows a stage report of its own, the walk is deleted
and this route reads that report; until then this is the only place a per-stage
duration exists.

`PROVIEW_STATUS_HOST` overrides the status cluster host for the walk
(default `pub-status.secu100.net`), so a run can be pointed at a controlled
host to prove which stage a failure lands in.

## 6. What the page shows

The live page prints `Failed at <stage> after <ms> ms: <detail>` where it used
to print "Camera offline", keeps the provider message in the tooltip line, and
offers **Copy the stage detail**, which copies that line plus one line per stage
with its own duration.
