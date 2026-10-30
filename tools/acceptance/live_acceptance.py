#!/usr/bin/env python3
"""One command that proves or fails the live-cloud claim, with numbers."""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import fcntl
import hashlib
import http.client
import json
import os
import socket
import struct
import subprocess
import sys
import time
from pathlib import Path

SCHEMA = "proview-live-acceptance/1"
GAP_LIMIT = 5.0
FIRST_FRAME_LIMIT = 30.0
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
EXIT_STAGE = 1
EXIT_PREFLIGHT = 2
EXIT_LAN = 3
EXIT_RELAY = 4


class StageError(Exception):
    def __init__(self, stage: str, detail: str, code: int = EXIT_STAGE):
        super().__init__(f"{stage}: {detail}")
        self.stage = stage
        self.detail = detail
        self.code = code


def stamp(moment: dt.datetime) -> str:
    return moment.strftime("%Y-%m-%dT%H:%M:%SZ")


def repo_root() -> Path:
    return Path(__file__).resolve().parents[2]


def git_text(root: Path, *args: str) -> str:
    done = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True)
    return done.stdout.strip() if done.returncode == 0 else ""


def read_env_file(path: str) -> dict:
    values = {}
    file = Path(path).expanduser()
    if not file.is_file():
        return values
    for line in file.read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        values[key.strip()] = value.strip().strip('"').strip("'")
    return values


def free_port() -> int:
    probe = socket.socket()
    try:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]
    finally:
        probe.close()


def guard_lan(target: str) -> dict:
    host, _, port = target.rpartition(":")
    reachable = False
    try:
        with socket.create_connection((host, int(port)), timeout=3.0):
            reachable = True
        detail = f"{target} accepted a TCP connection, so this host sits on the camera LAN"
    except (OSError, ValueError) as error:
        detail = f"{target} refused: {error.__class__.__name__}: {error}"
    return {"target": target, "reachable": reachable, "detail": detail}


def acquire_lock(path: str, wait: float):
    file = Path(path).expanduser()
    file.parent.mkdir(parents=True, exist_ok=True)
    handle = file.open("a+")
    deadline = time.monotonic() + wait
    while True:
        try:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return handle
        except BlockingIOError:
            if time.monotonic() >= deadline:
                handle.close()
                raise StageError(
                    "relay_lock",
                    f"{file} stayed held for {wait:.0f}s, another media session owns the relay",
                    EXIT_RELAY,
                )
            time.sleep(0.5)


class Http:
    def __init__(self, host: str, port: int):
        self.host = host
        self.port = port
        self.cookie = None

    def call(self, method: str, path: str, body=None, timeout: float = 60.0):
        conn = http.client.HTTPConnection(self.host, self.port, timeout=timeout)
        headers = {}
        payload = None
        if body is not None:
            payload = json.dumps(body).encode()
            headers["Content-Type"] = "application/json"
        if self.cookie:
            headers["Cookie"] = self.cookie
        try:
            conn.request(method, path, body=payload, headers=headers)
            response = conn.getresponse()
            data = response.read()
            status = response.status
            raw_cookie = response.getheader("Set-Cookie") or ""
        finally:
            conn.close()
        if "pvw_session=" in raw_cookie:
            self.cookie = raw_cookie.split(";")[0].strip()
        parsed = None
        try:
            parsed = json.loads(data)
        except ValueError:
            parsed = None
        return status, parsed, data


class Live:
    def __init__(self, host: str, port: int, path: str, cookie, timeout: float = 1.0):
        self.sock = socket.create_connection((host, port), timeout=10.0)
        key = base64.b64encode(os.urandom(16)).decode()
        lines = [
            f"GET {path} HTTP/1.1",
            f"Host: {host}:{port}",
            "Upgrade: websocket",
            "Connection: Upgrade",
            f"Sec-WebSocket-Key: {key}",
            "Sec-WebSocket-Version: 13",
        ]
        if cookie:
            lines.append(f"Cookie: {cookie}")
        self.sock.sendall(("\r\n".join(lines) + "\r\n\r\n").encode())
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = self.sock.recv(4096)
            if not chunk:
                raise StageError("live", "the socket closed during the websocket handshake")
            head += chunk
        header_block, _, rest = head.partition(b"\r\n\r\n")
        self.pending = bytearray(rest)
        head_text = header_block.decode(errors="replace")
        status_line = head_text.splitlines()[0] if head_text else ""
        if " 101 " not in status_line:
            raise StageError("live", f"the websocket handshake was refused: {status_line}")
        expected = base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()
        if expected.lower() not in head_text.lower():
            raise StageError("live", "the websocket handshake echoed the wrong Sec-WebSocket-Accept")
        self.sock.settimeout(timeout)

    def _frame_size(self):
        if len(self.pending) < 2:
            return None
        length = self.pending[1] & 0x7F
        header = 2
        if length == 126:
            header += 2
        elif length == 127:
            header += 8
        if len(self.pending) < header:
            return None
        if length == 126:
            length = struct.unpack("!H", bytes(self.pending[2:4]))[0]
        elif length == 127:
            length = struct.unpack("!Q", bytes(self.pending[2:10]))[0]
        if self.pending[1] & 0x80:
            header += 4
        return header + length

    def _fill(self) -> None:
        chunk = self.sock.recv(65536)
        if not chunk:
            raise ConnectionError("the peer closed the socket")
        self.pending.extend(chunk)

    def recv(self):
        while True:
            size = self._frame_size()
            if size is not None and len(self.pending) >= size:
                break
            self._fill()
        frame = bytes(self.pending[:size])
        del self.pending[:size]
        opcode = frame[0] & 0x0F
        offset = 2
        length = frame[1] & 0x7F
        if length == 126:
            length = struct.unpack("!H", frame[2:4])[0]
            offset = 4
        elif length == 127:
            length = struct.unpack("!Q", frame[2:10])[0]
            offset = 10
        if frame[1] & 0x80:
            mask = frame[offset:offset + 4]
            body = frame[offset + 4:]
            payload = bytes(byte ^ mask[index % 4] for index, byte in enumerate(body))
        else:
            payload = frame[offset:]
        return opcode, payload

    def send(self, opcode: int, payload: bytes = b"") -> None:
        mask = os.urandom(4)
        head = bytearray([0x80 | opcode])
        count = len(payload)
        if count < 126:
            head.append(0x80 | count)
        elif count < 65536:
            head.append(0x80 | 126)
            head.extend(struct.pack("!H", count))
        else:
            head.append(0x80 | 127)
            head.extend(struct.pack("!Q", count))
        body = bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))
        self.sock.sendall(bytes(head) + mask + body)

    def close(self) -> None:
        try:
            self.send(0x8)
        except OSError:
            pass
        try:
            self.sock.close()
        except OSError:
            pass


def build_binary(root: Path, args, identity: dict):
    binary = Path(args.bin).expanduser() if args.bin else root / "target" / "release" / "proview-server"
    if args.no_build:
        if not binary.is_file():
            raise StageError("build", f"{binary} is missing and --no-build was passed", EXIT_PREFLIGHT)
        info = binary_provenance(binary, reused=True)
        stamp_commit = git_text(root, "log", "-1", "--format=%ct", "HEAD")
        commit_time = int(stamp_commit) if stamp_commit.isdigit() else 0
        info["commit_at"] = (
            dt.datetime.fromtimestamp(commit_time, dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
            if commit_time
            else None
        )
        if args.mode == "cloud" and commit_time and info["mtime_epoch"] + 1 < commit_time:
            raise StageError(
                "build",
                f"--no-build would label {binary} (built {info['mtime']}) as commit {info['commit_at']};"
                " rebuild the branch or point --bin at a binary built after it",
            )
        return binary, info
    slot = Path(args.build_slot).expanduser()
    if not slot.is_file():
        raise StageError("build", f"{slot} is missing, so the harness will not build unscheduled", EXIT_PREFLIGHT)
    command = ["bash", str(slot), "cargo", "build", "--release", "-p", "proview-server"]
    env = {key: value for key, value in os.environ.items() if not key.startswith("PROVIEW_")}
    env.update(identity)
    if args.mode == "cloud":
        env["PROVIEW_REQUIRE_CLOUD_IDENTITY"] = "1"
    started = time.monotonic()
    done = subprocess.run(command, cwd=str(root), capture_output=True, text=True, env=env)
    spent = round(time.monotonic() - started, 2)
    if done.returncode != 0:
        tail = "\n".join((done.stderr or done.stdout or "").splitlines()[-25:])
        raise StageError("build", f"cargo build --release -p proview-server failed in {spent}s:\n{tail}")
    if not binary.is_file():
        raise StageError("build", f"the build succeeded but {binary} does not exist")
    info = binary_provenance(binary, reused=False)
    info["seconds"] = spent
    return binary, info


def binary_provenance(binary: Path, reused: bool) -> dict:
    modified = binary.stat().st_mtime
    return {
        "reused": reused,
        "binary": str(binary),
        "bytes": binary.stat().st_size,
        "mtime": dt.datetime.fromtimestamp(modified, dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "mtime_epoch": modified,
    }


def server_env(args, port: int, data_dir: Path) -> dict:
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": os.environ.get("HOME", "/root"),
        "PROVIEW_LISTEN": f"127.0.0.1:{port}",
        "PROVIEW_DATA_DIR": str(data_dir),
        "PROVIEW_MODE": "cloud",
        "ROCKET_SECRET_KEY": base64.b64encode(os.urandom(32)).decode(),
        "RUST_LOG": "info",
    }
    if args.mode == "fixture":
        env["PROVIEW_PROVIDER"] = "fixture"
        env["PROVIEW_FIXTURE_DIR"] = str(Path(args.fixture_dir).expanduser())
    device_file = Path(args.device_env_file).expanduser()
    if args.mode == "cloud" and device_file.is_file():
        device = read_env_file(str(device_file))
        if "PROVIEW_DEVICE_USER" in device and "PROVIEW_DEVICE_PASS" in device:
            env["PROVIEW_DEVICE_USER"] = device["PROVIEW_DEVICE_USER"]
            env["PROVIEW_DEVICE_PASS"] = device["PROVIEW_DEVICE_PASS"]
    if args.mode == "cloud" and args.sn:
        env["PROVIEW_DEVICE_SERIAL"] = args.sn
    return env


def start_server(binary: Path, root: Path, env: dict, log_path: Path):
    log = log_path.open("wb")
    process = subprocess.Popen([str(binary)], cwd=str(root), env=env, stdout=log, stderr=subprocess.STDOUT)
    return process, log


def wait_ready(process, http: Http, deadline: float) -> float:
    started = time.monotonic()
    while time.monotonic() - started < deadline:
        if process.poll() is not None:
            raise StageError("serve", f"the server exited with code {process.returncode} before answering /api/health")
        try:
            status, _, _ = http.call("GET", "/api/health", timeout=2.0)
            if status == 200:
                return round(time.monotonic() - started, 3)
        except OSError:
            pass
        time.sleep(0.2)
    raise StageError("serve", f"/api/health did not answer within {deadline:.0f}s")


def classify(payload: bytes, seen_init: bool) -> str:
    if not seen_init:
        return "init"
    trun = payload.find(b"trun")
    if trun < 0 or trun + 32 > len(payload):
        return "fragment"
    flags = int.from_bytes(payload[trun + 28:trun + 32], "big")
    return "keyframe" if flags & 0x02000000 else "delta"


def run_live(live: Live, http: Http, args, stats_path: str, dump_dir: Path, report: dict) -> dict:
    started = time.monotonic()
    next_sample = started + args.sample
    frames = 0
    keyframes = 0
    deltas = 0
    inits = 0
    fragments = 0
    other = 0
    bytes_total = 0
    first_frame_at = None
    last_frame_at = None
    longest_gap = 0.0
    samples = []
    stalls = []
    hello = None
    server_last = None
    dumped = bytearray()
    dump_rows = []
    waiting_keyframe = True

    while True:
        elapsed = time.monotonic() - started
        if elapsed >= args.duration:
            break
        try:
            opcode, payload = live.recv()
        except TimeoutError:
            if first_frame_at is None and elapsed > FIRST_FRAME_LIMIT:
                raise StageError(
                    "live",
                    f"no websocket frame arrived within {FIRST_FRAME_LIMIT:.0f}s of the handshake",
                )
            if last_frame_at is not None:
                gap = time.monotonic() - last_frame_at
                if gap > GAP_LIMIT:
                    stalls.append({"at_s": round(elapsed, 3), "gap_s": round(gap, 3)})
                    break
            continue
        except ConnectionError as error:
            raise StageError("live", f"the server closed the socket after {elapsed:.1f}s: {error}")
        moment = time.monotonic()
        if opcode == 0x9:
            live.send(0xA, payload)
            continue
        if opcode == 0x8:
            raise StageError("live", f"the server sent a close frame after {elapsed:.1f}s")
        if opcode == 0x1:
            if hello is None:
                try:
                    hello = json.loads(payload)
                except ValueError:
                    hello = {"raw": payload.decode(errors="replace")[:200]}
            continue
        if opcode != 0x2:
            continue
        kind = classify(payload, inits > 0)
        if first_frame_at is None:
            first_frame_at = moment
        if last_frame_at is not None:
            longest_gap = max(longest_gap, moment - last_frame_at)
        last_frame_at = moment
        frames += 1
        bytes_total += len(payload)
        if kind == "keyframe":
            keyframes += 1
        elif kind == "delta":
            deltas += 1
        elif kind == "init":
            inits += 1
        elif kind == "fragment":
            fragments += 1
        else:
            other += 1
        if len(dumped) < args.dump_bytes:
            if kind in ("init", "keyframe", "fragment"):
                waiting_keyframe = False
            if not waiting_keyframe:
                dumped.extend(payload)
                dump_rows.append((kind, 0, len(payload), round(moment - started, 3)))
        now = time.monotonic()
        if now >= next_sample:
            status, stats, _ = http.call("GET", stats_path, timeout=10.0)
            streams = stats.get("streams") if status == 200 and isinstance(stats, dict) else None
            if isinstance(streams, list) and streams:
                server_last = streams[0]
            samples.append(
                {
                    "at_s": round(now - started, 3),
                    "frames": frames,
                    "bytes": bytes_total,
                    "fps_avg": round(frames / (now - started), 3) if now > started else 0.0,
                    "longest_gap_s": round(longest_gap, 3),
                    "server_keyframes": (server_last or {}).get("keyframes"),
                    "server_packets": (server_last or {}).get("packets"),
                    "server_bytes": (server_last or {}).get("bytes"),
                    "server_mode": (server_last or {}).get("mode"),
                    "server_publishers": (server_last or {}).get("publishers"),
                    "server_subscribers": (server_last or {}).get("subscribers"),
                }
            )
            next_sample += args.sample

    codec = (hello or {}).get("codec") if isinstance(hello, dict) else None
    dump_name, demuxer = "frames.mp4", "mp4"
    if dumped:
        (dump_dir / dump_name).write_bytes(bytes(dumped))
    if dump_rows:
        (dump_dir / "frames.tsv").write_text(
            "\n".join(f"{kind}\t{pts}\t{size}\t{at}" for kind, pts, size, at in dump_rows) + "\n"
        )
    spent = time.monotonic() - started
    server_keyframes = (server_last or {}).get("keyframes")
    report["live"] = {
        "hello": hello,
        "framing": "fmp4",
        "unit": "fragment",
        "frames": frames,
        "keyframes_ws": keyframes,
        "deltas": deltas,
        "inits": inits,
        "fragments": fragments,
        "other": other,
        "bytes": bytes_total,
        "seconds": round(spent, 3),
        "fps_avg": round(frames / spent, 3) if spent > 0 else 0.0,
        "mb": round(bytes_total / 1048576, 3),
        "first_frame_s": round(first_frame_at - started, 3) if first_frame_at else None,
        "longest_gap_s": round(longest_gap, 3),
        "stalls": stalls,
        "samples": samples,
        "server": server_last,
        "server_keyframes": server_keyframes,
        "dump": {
            "dir": str(dump_dir),
            "file": str(dump_dir / dump_name),
            "bytes": len(dumped),
            "demuxer": demuxer,
            "codec": codec,
            "ffprobe": (
                f"ffprobe -v error -f {demuxer} -count_frames"
                f" -show_entries stream=codec_name,width,height,nb_read_frames {dump_dir / dump_name}"
            ),
        },
    }
    if frames == 0:
        raise StageError("live", f"no video frame arrived in {spent:.1f}s of the run")
    if stalls:
        first = stalls[0]
        raise StageError(
            "live",
            f"the stream stalled for {first['gap_s']:.1f}s at {first['at_s']:.1f}s, past the {GAP_LIMIT:.0f}s limit",
        )
    if spent < args.duration - 1.0:
        raise StageError("live", f"the run stopped after {spent:.1f}s of the requested {args.duration:.0f}s")
    authoritative = keyframes or server_keyframes
    if not authoritative:
        raise StageError(
            "keyframes",
            f"{frames} frames and {bytes_total} bytes arrived but no keyframe was counted on the socket"
            f" (server counter: {server_keyframes})",
        )
    return report["live"]


def live_path(args, device: str) -> str:
    query = f"device={device}&channel={args.channel}&quality={args.quality}"
    if args.codec:
        query += f"&codec={args.codec}"
    return f"/ws/live?{query}"


def render(report: dict) -> str:
    lines = []
    lines.append(f"# Live acceptance {report['verdict'].upper()}")
    lines.append("")
    lines.append(f"- run: {report['run_id']}")
    lines.append(f"- branch: `{report['git']['branch']}` at `{report['git']['sha']}`")
    if report["git"]["dirty"]:
        lines.append(f"- dirty files: {len(report['git']['dirty'])}")
    lines.append(f"- mode: {report['mode']} · duration {report['duration']}s · samples every {report['sample']}s")
    lines.append(f"- host: {report['host']['name']} · load {report['host']['load']} · camera LAN {report['lan']['detail']}")
    verdict_line = f"- verdict: **{report['verdict']}**, exit {report['exit_code']}"
    if report.get("failed_stage"):
        verdict_line += f", stopped at stage {report['failed_stage']}"
    lines.append(verdict_line)
    build = report["server"].get("build") or {}
    if build:
        state = "reused" if build.get("reused") else "built"
        lines.append(f"- binary: {build.get('binary')} · {state} · {build.get('mtime')}")
    if report.get("failure"):
        lines.append(f"- failure: {report['failure']}")
    lines.append("")
    lines.append("## Stages")
    lines.append("")
    lines.append("| stage | ok | seconds | detail |")
    lines.append("|---|---|---|---|")
    for stage in report["stages"]:
        detail = str(stage["detail"]).replace("|", "\\|")
        lines.append(f"| {stage['name']} | {'yes' if stage['ok'] else 'no'} | {stage.get('seconds', '')} | {detail} |")
    lines.append("")
    live = report.get("live")
    if live:
        lines.append("## Numbers")
        lines.append("")
        lines.append("| metric | value |")
        lines.append("|---|---|")
        lines.append(f"| video messages | {live['frames']} ({live['unit']}) |")
        lines.append(f"| socket framing | {live['framing']} |")
        lines.append(f"| socket codec | {live.get('codec') or 'unknown'} |")
        server_mode = (live.get('server') or {}).get('mode')
        lines.append(f"| server stream mode | {server_mode or 'unknown'} |")
        lines.append(f"| keyframes seen on the socket | {live['keyframes_ws']} |")
        lines.append(f"| keyframes reported by the server | {live['server_keyframes']} |")
        lines.append(f"| deltas | {live['deltas']} |")
        lines.append(f"| init segments | {live['inits']} |")
        lines.append(f"| payload bytes | {live['bytes']} ({live['mb']} MB) |")
        lines.append(f"| seconds measured | {live['seconds']} |")
        lines.append(f"| fps average | {live['fps_avg']} |")
        lines.append(f"| first frame | {live['first_frame_s']} s after the handshake |")
        lines.append(f"| longest gap | {live['longest_gap_s']} s |")
        lines.append(f"| stalls past {GAP_LIMIT:.0f}s | {len(live['stalls'])} |")
        lines.append("")
        if live["samples"]:
            lines.append("## Samples")
            lines.append("")
            lines.append("| at s | frames | bytes | fps avg | longest gap s | server keyframes |")
            lines.append("|---|---|---|---|---|---|")
            for sample in live["samples"]:
                lines.append(
                    f"| {sample['at_s']} | {sample['frames']} | {sample['bytes']} | {sample['fps_avg']} | "
                    f"{sample['longest_gap_s']} | {sample['server_keyframes']} |"
                )
            lines.append("")
        lines.append("## Payload dump")
        lines.append("")
        lines.append(f"- {live['dump']['bytes']} payload bytes in `{live['dump']['file']}`")
        lines.append(f"- verify outside the harness: `{live['dump']['ffprobe']}`")
        lines.append("")
    http_stages = report.get("http")
    if http_stages:
        lines.append("## HTTP evidence")
        lines.append("")
        for name, value in http_stages.items():
            if not isinstance(value, dict):
                lines.append(f"- {name}: {value}")
                continue
            summary = {key: item for key, item in value.items() if key != "body"}
            lines.append(f"- {name}: status {value.get('status')} · {json.dumps(summary)}")
        lines.append("")
    lines.append("## Limits")
    lines.append("")
    lines.append("- A pass here proves the cloud relay carried video for the whole run. It does not prove parity with the vendor app.")
    lines.append("- A pass counts only in cloud mode, with the LAN guard reporting the camera unreachable; a reachable camera turns the verdict into lan-reachable and exits 3.")
    lines.append("- pass-fixture comes from a recorded clip: it proves the counter, never the cloud.")
    lines.append("- The harness never retries a busy relay, never runs two sessions at once, and never edits a crate.")
    lines.append("")
    return "\n".join(lines)


def main(argv) -> int:
    parser = argparse.ArgumentParser(description="Prove or fail the live-cloud claim in one command.")
    parser.add_argument("--repo", default="")
    parser.add_argument("--mode", choices=["cloud", "fixture"], default="cloud")
    parser.add_argument("--duration", type=float, default=300.0)
    parser.add_argument("--sample", type=float, default=10.0)
    parser.add_argument("--device", default="")
    parser.add_argument("--sn", default="")
    parser.add_argument("--channel", type=int, default=1)
    parser.add_argument("--quality", choices=["main", "sub"], default="main")
    parser.add_argument("--codec", default="")
    parser.add_argument("--lan-target", default="192.168.0.100:554")
    parser.add_argument("--allow-lan", action="store_true")
    parser.add_argument("--env-file", default="~/re/proview-cctv/secrets/proview.env")
    parser.add_argument("--identity-file", default="~/re/proview-cctv/secrets/identity.env")
    parser.add_argument("--device-env-file", default="~/re/proview-cctv/secrets/device.env")
    parser.add_argument("--lock", default="~/re/proview-cctv/relay.lock")
    parser.add_argument("--lock-wait", type=float, default=60.0)
    parser.add_argument("--out", default="~/re/proview-cctv/acceptance")
    parser.add_argument("--build-slot", default="~/re/proview-cctv/build-slot.sh")
    parser.add_argument("--bin", default="")
    parser.add_argument("--no-build", action="store_true")
    parser.add_argument("--dump-bytes", type=int, default=4194304)
    parser.add_argument("--fixture-dir", default="")
    parser.add_argument("--keep-server", action="store_true")
    args = parser.parse_args(argv)

    root = Path(args.repo).expanduser().resolve() if args.repo else repo_root()
    started = dt.datetime.now(dt.timezone.utc)
    prefix = git_text(root, "rev-parse", "--short=12", "HEAD") or "unknown"
    run_id = f"{prefix}-{started.strftime('%Y%m%dT%H%M%SZ')}"
    out_dir = Path(args.out).expanduser() / run_id
    out_dir.mkdir(parents=True, exist_ok=True)
    dump_dir = out_dir / "dump"
    dump_dir.mkdir(exist_ok=True)

    report = {
        "schema": SCHEMA,
        "run_id": run_id,
        "mode": args.mode,
        "duration": args.duration,
        "sample": args.sample,
        "started_at": stamp(started),
        "verdict": "fail",
        "failed_stage": None,
        "failure": None,
        "git": {
            "sha": git_text(root, "rev-parse", "HEAD"),
            "branch": git_text(root, "rev-parse", "--abbrev-ref", "HEAD"),
            "dirty": [line for line in git_text(root, "status", "--porcelain").splitlines() if line],
        },
        "host": {
            "name": socket.gethostname(),
            "load": os.getloadavg(),
            "kernel": os.uname().release,
            "python": sys.version.split()[0],
        },
        "lan": {"target": args.lan_target, "reachable": None, "detail": "not checked"},
        "stages": [],
        "http": {},
        "server": {},
        "live": None,
    }

    marks = {"at": time.monotonic()}

    def tick() -> float:
        marks["at"] = time.monotonic()
        return marks["at"]

    def stage(name: str, ok: bool, detail, seconds=None) -> None:
        report["stages"].append({"name": name, "ok": ok, "detail": detail, "seconds": seconds})

    process = None
    log = None
    lock = None
    live = None
    exit_code = 0
    try:
        clock = tick()
        if args.mode == "cloud":
            lan = guard_lan(args.lan_target)
            report["lan"] = lan
            stage("lan_guard", not lan["reachable"], lan["detail"], round(time.monotonic() - clock, 3))
            if lan["reachable"] and not args.allow_lan:
                raise StageError(
                    "lan_guard",
                    f"the camera answers at {args.lan_target}, so a stream here would not prove the cloud path",
                    EXIT_LAN,
                )
        else:
            report["lan"] = {"target": args.lan_target, "reachable": None, "detail": "skipped in fixture mode"}
            stage("lan_guard", True, "skipped in fixture mode", 0.0)

        clock = tick()
        identity = read_env_file(args.identity_file)
        wanted = ("PROVIEW_APP_UUID", "PROVIEW_APP_KEY", "PROVIEW_APP_SECRET")
        present = [key for key in wanted if identity.get(key)]
        report["identity"] = {"file": args.identity_file, "present": present}
        if args.mode == "cloud" and len(present) != len(wanted):
            missing = [key for key in wanted if key not in present]
            raise StageError(
                "identity",
                f"{args.identity_file} must define {', '.join(missing)}; without them the build cannot sign the vendor envelope",
                EXIT_PREFLIGHT,
            )
        stage("identity", True, f"{len(present)}/3 keys from {args.identity_file}", round(time.monotonic() - clock, 3))

        clock = tick()
        binary, build_info = build_binary(root, args, identity)
        report["server"]["build"] = build_info
        stage(
            "build",
            True,
            f"{build_info['binary']} · {'reused' if build_info['reused'] else 'built'} {build_info['mtime']}",
            round(time.monotonic() - clock, 3),
        )

        clock = tick()
        credentials = read_env_file(args.env_file)
        email = credentials.get("PROVIEW_EMAIL")
        password = credentials.get("PROVIEW_PASSWORD")
        if args.mode == "cloud" and (not email or not password):
            raise StageError(
                "credentials",
                f"{args.env_file} must define PROVIEW_EMAIL and PROVIEW_PASSWORD",
                EXIT_PREFLIGHT,
            )
        if args.mode == "fixture":
            account = json.loads((Path(args.fixture_dir).expanduser() / "account.json").read_text())
            email = account.get("email", "")
            password = account.get("password", "")
        source = args.env_file if args.mode == "cloud" else args.fixture_dir
        stage("credentials", True, f"sign-in material present from {source}", round(time.monotonic() - clock, 3))

        clock = tick()
        if args.mode == "cloud":
            lock = acquire_lock(args.lock, args.lock_wait)
            stage("relay_lock", True, f"exclusive lock on {Path(args.lock).expanduser()}", round(time.monotonic() - clock, 3))
        else:
            stage("relay_lock", True, "skipped in fixture mode, no relay session", 0.0)

        port = free_port()
        env = server_env(args, port, out_dir / "state")
        clock = tick()
        process, log = start_server(binary, root, env, out_dir / "server.log")
        http = Http("127.0.0.1", port)
        ready = wait_ready(process, http, 60.0)
        report["server"]["listen"] = f"127.0.0.1:{port}"
        report["server"]["log"] = str(out_dir / "server.log")
        stage("serve", True, f"127.0.0.1:{port} answered /api/health in {ready}s", round(time.monotonic() - clock, 3))

        clock = tick()
        status, body, raw = http.call("POST", "/api/login", {"email": email, "password": password}, timeout=120.0)
        message = body.get("message") if isinstance(body, dict) else raw[:200].decode(errors="replace")
        report["http"]["login"] = {"status": status, "message": message}
        if status != 200 or not (isinstance(body, dict) and body.get("ok")):
            raise StageError("login", f"POST /api/login returned {status}: {message}")
        stage("login", True, f"200 · {message}", round(time.monotonic() - clock, 3))

        clock = tick()
        status, mode_body, _ = http.call("GET", "/api/config/mode", timeout=30.0)
        modes = json.dumps(mode_body) if isinstance(mode_body, dict) else ""
        if args.mode == "cloud":
            resolved = mode_body if isinstance(mode_body, dict) else {}
            if resolved.get("mode") != "cloud" or resolved.get("source") != "cloud":
                raise StageError(
                    "mode",
                    f"the server resolved mode {modes or status}, so this run would not exercise the cloud provider",
                )
        stage("mode", True, modes or f"status {status}", round(time.monotonic() - clock, 3))

        clock = tick()
        status, devices, raw = http.call("GET", "/api/devices", timeout=180.0)
        count = len(devices) if isinstance(devices, list) else 0
        text = raw.decode(errors="replace")
        sn_match = None
        if args.sn:
            sn_match = args.sn.lower() in text.lower()
        ids = [device.get("id") for device in devices if isinstance(device, dict)] if isinstance(devices, list) else []
        report["http"]["devices"] = {
            "status": status,
            "count": count,
            "sn": args.sn or None,
            "sn_match": sn_match,
            "ids": ids,
        }
        if status != 200 or count == 0:
            raise StageError("devices", f"GET /api/devices returned {status} with {count} devices")
        if args.sn and not sn_match:
            raise StageError("devices", f"GET /api/devices did not carry the serial {args.sn}")
        stage("devices", True, f"200 · {count} devices · ids {ids}", round(time.monotonic() - clock, 3))

        device = args.device or (devices[0].get("id") if isinstance(devices[0], dict) else "")

        clock = tick()
        status, ticket, raw = http.call(
            "GET",
            f"/api/stream?device={device}&channel={args.channel}&quality={args.quality}",
            timeout=180.0,
        )
        detail = ticket if isinstance(ticket, dict) else raw[:300].decode(errors="replace")
        report["http"]["stream"] = {
            "status": status,
            "ticket_keyframes": (ticket or {}).get("keyframes") if isinstance(ticket, dict) else None,
            "body": detail,
        }
        if status != 200:
            raise StageError("stream", f"GET /api/stream returned {status}: {detail}")
        ticket_source = ticket.get("source") if isinstance(ticket, dict) else None
        report["http"]["stream"]["ticket_source"] = ticket_source
        if args.mode == "cloud" and ticket_source != "cloud":
            raise StageError(
                "stream",
                f"the ticket came from the {ticket_source} provider, not the cloud, so a stream here would prove nothing",
            )
        stage(
            "stream",
            True,
            f"200 · source {ticket_source} · {json.dumps(detail)[:180]}",
            round(time.monotonic() - clock, 3),
        )

        clock = tick()
        path = live_path(args, device)
        report["http"]["live_url"] = path
        live = Live("127.0.0.1", port, path, http.cookie, timeout=1.0)
        stage("websocket", True, f"101 switching protocols on {path}", round(time.monotonic() - clock, 3))

        clock = tick()
        numbers = run_live(live, http, args, "/api/live/stats", dump_dir, report)
        stage(
            "live",
            True,
            f"{numbers['frames']} {numbers['unit']}s · {numbers['bytes']} bytes · {numbers['fps_avg']} per second · gap {numbers['longest_gap_s']}s",
            round(time.monotonic() - clock, 3),
        )
        authoritative = numbers["keyframes_ws"] or numbers["server_keyframes"] or 0
        source = "socket" if numbers["keyframes_ws"] else "server /api/live/stats"
        stage("keyframes", True, f"{authoritative} keyframes from the {source} over {numbers['seconds']}s", 0.0)
        if args.mode == "cloud" and report["lan"]["reachable"]:
            report["verdict"] = "lan-reachable"
            exit_code = EXIT_LAN
        elif args.mode == "cloud":
            report["verdict"] = "pass"
        else:
            report["verdict"] = "pass-fixture"
    except StageError as error:
        report["verdict"] = "fail"
        report["failed_stage"] = error.stage
        report["failure"] = error.detail
        exit_code = error.code
        if args.mode == "cloud" and report["lan"]["reachable"] and error.stage != "lan_guard":
            report["verdict"] = "lan-reachable"
            exit_code = EXIT_LAN
        stage(error.stage, False, error.detail, round(time.monotonic() - marks["at"], 3))
    except (OSError, ValueError, KeyError) as error:
        report["verdict"] = "fail"
        report["failed_stage"] = "harness"
        report["failure"] = f"{error.__class__.__name__}: {error}"
        exit_code = EXIT_PREFLIGHT
        stage("harness", False, report["failure"])
    finally:
        if live is not None:
            live.close()
        if process is not None and not args.keep_server:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
        if log is not None:
            log.close()
        if process is not None:
            report["server"]["exit_code"] = process.returncode
        if lock is not None:
            fcntl.flock(lock, fcntl.LOCK_UN)
            lock.close()

    report["finished_at"] = stamp(dt.datetime.now(dt.timezone.utc))
    report["exit_code"] = exit_code
    (out_dir / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    (out_dir / "report.md").write_text(render(report))

    lan_state = "unreachable" if report["lan"]["reachable"] is False else ("reachable" if report["lan"]["reachable"] else "not checked")
    print(f"verdict: {report['verdict']} · mode: {args.mode} · camera LAN: {lan_state}")
    if report["failure"]:
        print(f"stage: {report['failed_stage']}")
        print(f"detail: {report['failure']}")
    live_numbers = report.get("live")
    if live_numbers:
        print(
            f"frames: {live_numbers['frames']} · bytes: {live_numbers['bytes']} · fps: {live_numbers['fps_avg']}"
            f" · longest gap: {live_numbers['longest_gap_s']}s · seconds: {live_numbers['seconds']}"
        )
    print(f"report: {out_dir / 'report.md'}")
    return exit_code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
