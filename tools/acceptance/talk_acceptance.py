#!/usr/bin/env python3
import argparse
import base64
import json
import os
import select
import socket
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from live_acceptance import (  # noqa: E402
    Http,
    Live,
    StageError,
    acquire_lock,
    free_port,
    guard_lan,
    wait_ready,
)

IDENTITY_KEYS = ("PROVIEW_APP_UUID", "PROVIEW_APP_KEY", "PROVIEW_APP_SECRET")
FORBIDDEN_KEYS = ("PROVIEW_DEVICE_USER", "PROVIEW_DEVICE_PASS")


def talk_row(body):
    for row in body.get("capabilities", []):
        if row.get("feature") == "two-way audio":
            return row
    return None


def main(argv):
    parser = argparse.ArgumentParser(description="Measure two-way audio over the vendor relay without a camera password")
    parser.add_argument("--binary", required=True)
    parser.add_argument("--sn", required=True)
    parser.add_argument("--channel", type=int, default=None)
    parser.add_argument("--seconds", type=float, default=3.0)
    parser.add_argument("--lan-guard", default="192.168.0.100:34567")
    parser.add_argument("--relay-lock", default="~/re/proview-cctv/relay.lock")
    parser.add_argument("--out", default="~/re/proview-cctv/acceptance/talk")
    args = parser.parse_args(argv)

    out = Path(args.out).expanduser()
    out.mkdir(parents=True, exist_ok=True)
    report = {"binary": str(Path(args.binary).expanduser()), "serial": args.sn, "stages": {}, "numbers": {}}
    clock = time.monotonic()

    def stage(name, ok, detail):
        report["stages"][name] = {
            "ok": bool(ok),
            "detail": detail,
            "seconds": round(time.monotonic() - clock, 3),
        }
        print(f"{name:12s} {'ok' if ok else 'FAIL'}  {detail}", flush=True)
        if not ok:
            raise StageError(name, detail)

    lock = None
    process = None
    live = None
    try:
        guard = guard_lan(args.lan_guard)
        stage("lan_guard", not guard["reachable"], guard["detail"])

        identity = {key: os.environ.get(key, "").strip() for key in IDENTITY_KEYS}
        missing = [key for key, value in identity.items() if not value]
        stage("identity", not missing, "complete" if not missing else f"missing {', '.join(missing)}")

        account = os.environ.get("PROVIEW_EMAIL", "").strip()
        password = os.environ.get("PROVIEW_PASSWORD", "").strip()
        stage("credentials", bool(account and password), "cloud sign-in material from the environment")

        lock = acquire_lock(args.relay_lock, 600.0)
        stage("relay_lock", True, f"exclusive lock on {Path(args.relay_lock).expanduser()}")

        port = free_port()
        data_dir = out / "state"
        data_dir.mkdir(exist_ok=True)
        env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": os.environ.get("HOME", "/root"),
            "PROVIEW_LISTEN": f"127.0.0.1:{port}",
            "PROVIEW_DATA_DIR": str(data_dir),
            "PROVIEW_MODE": "cloud",
            "PROVIEW_DEVICE_SERIAL": args.sn,
            "ROCKET_SECRET_KEY": base64.b64encode(os.urandom(32)).decode(),
            "RUST_LOG": "info",
        }
        env.update(identity)
        leaked = [key for key in FORBIDDEN_KEYS if key in env or os.environ.get(key)]
        stage(
            "no_device_credentials",
            not leaked,
            "the server runs without PROVIEW_DEVICE_USER/PASS"
            if not leaked
            else f"a camera credential is set for the server: {', '.join(leaked)}",
        )

        log_path = out / "server.log"
        log = log_path.open("wb")
        process = subprocess.Popen(
            [str(Path(args.binary).expanduser())],
            cwd=str(Path(args.binary).expanduser().resolve().parent),
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        http = Http("127.0.0.1", port)
        ready = wait_ready(process, http, 45.0)
        stage("serve", True, f"127.0.0.1:{port} answered /api/health in {ready}s")

        status, body, raw = http.call("POST", "/api/login", {"email": account, "password": password}, timeout=180.0)
        if status != 200:
            stage("login", False, f"POST /api/login returned {status}: {raw[:200]!r}")
        stage("login", True, "signed in through the cloud")

        status, body, raw = http.call(
            "GET", f"/api/device/controls?device={args.sn}", timeout=120.0
        )
        if status != 200:
            stage("capability", False, f"GET /api/device/controls returned {status}: {raw[:200]!r}")
        row = talk_row(body or {})
        if row is None:
            stage("capability", False, "the report carries no two-way audio row")
        stage(
            "capability",
            row["state"] == "supported",
            f"two-way audio: {row['state']} — {row['evidence']}",
        )
        report["numbers"]["capability_evidence"] = row["evidence"]

        query = f"/ws/talk?device={args.sn}"
        if args.channel is not None:
            query += f"&channel={args.channel}"
        live = Live("127.0.0.1", port, query, http.cookie, timeout=30.0)
        opcode, payload = live.recv()
        if opcode != 1:
            stage("talk", False, f"the first message was opcode {opcode}, not a ready message")
        ready_message = json.loads(payload)
        if ready_message.get("kind") == "error":
            stage("talk", False, f"the route refused: {ready_message.get('message')}")
        stage(
            "talk",
            ready_message.get("kind") == "ready" and ready_message.get("path") == "relay",
            f"ready in {ready_message.get('claim_ms')} ms on the {ready_message.get('path')} path",
        )
        report["numbers"]["claim_ms"] = ready_message.get("claim_ms")
        report["numbers"]["path"] = ready_message.get("path")

        silence = b"\x00" * 640
        live.sock.settimeout(1.0)
        deadline = time.monotonic() + args.seconds
        chunks = 0
        next_send = time.monotonic()
        while time.monotonic() < deadline:
            if time.monotonic() >= next_send:
                live.send(0x2, silence)
                chunks += 1
                next_send += 0.04
            readable, _, _ = select.select(
                [live.sock], [], [], max(0.0, next_send - time.monotonic())
            )
            if not readable:
                continue
            try:
                opcode, payload = live.recv()
            except (socket.timeout, ConnectionError, OSError):
                continue
            if opcode == 8:
                break
        live.send(0x1, json.dumps({"type": "stop"}).encode())
        live.sock.settimeout(20.0)
        final = None
        while True:
            try:
                opcode, payload = live.recv()
            except (socket.timeout, ConnectionError, OSError):
                break
            if opcode == 1:
                message = json.loads(payload)
                if message.get("kind") == "report":
                    final = message
            elif opcode == 8:
                break
        live.close()
        live = None

        if final is None:
            stage("measure", False, "the route sent no closing report")
        report["numbers"].update(
            {
                "pcm_chunks": chunks,
                "frames": final["frames"],
                "bytes": final["bytes"],
                "dropped": final["dropped"],
                "seconds": final["seconds"],
                "frames_per_second": final["frames_per_second"],
                "write_ms_mean": final["write_ms_mean"],
                "write_ms_max": final["write_ms_max"],
                "inbound_bytes": final["inbound_bytes"],
                "inbound_frames": final["inbound_frames"],
            }
        )
        stage(
            "measure",
            final["frames"] > 0 and final["bytes"] > 0 and final["inbound_bytes"] > 0,
            f"{final['frames']} frames / {final['bytes']} bytes out in {final['seconds']:.2f} s, "
            f"{final['inbound_bytes']} bytes / {final['inbound_frames']} frames back from the camera",
        )
        report["verdict"] = "pass"
        return 0
    except StageError as error:
        report["verdict"] = "fail"
        report["failure"] = {"stage": error.stage, "detail": error.detail}
        return error.code
    finally:
        if live is not None:
            live.close()
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
        if lock is not None:
            lock.close()
        (out / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(f"report       {out / 'report.json'}")


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
