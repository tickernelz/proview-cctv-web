#!/usr/bin/env python3
"""Prove or fail the cloud PTZ claim: the panel may only enable Move when this passes."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import live_acceptance as base

SCHEMA = "proview-cloud-ptz/1"
EXIT_LAN = 3
MOVE_MIN_PX = 8
STILL_MAX_PX = 3


def now_stamp(moment: dt.datetime) -> str:
    return moment.strftime("%Y-%m-%dT%H:%M:%SZ")


def load_views():
    try:
        import numpy
        from PIL import Image
    except ImportError as error:
        raise base.StageError(
            "preflight",
            f"numpy and pillow are required to measure pixels ({error}); use ~/re/proview-cctv/.venv/bin/python",
            base.EXIT_PREFLIGHT,
        )
    return numpy, Image


def halves(numpy, array):
    height = array.shape[0] // 2
    return {"top": array[:height], "bottom": array[height : height * 2]}


TILE_HEIGHT = 60
TILE_WIDTH = 80
TEXT_HEIGHT = 110
TILE_COUNT = 8
MOVED_TILE_PX = 5


def tile_shift(numpy, before, after):
    height, width = before.shape
    spectrum_before = numpy.fft.rfft2(before - before.mean())
    spectrum_after = numpy.fft.rfft2(after - after.mean())
    cross = spectrum_after * numpy.conj(spectrum_before)
    cross /= numpy.abs(cross) + 1e-12
    correlation = numpy.fft.irfft2(cross, s=before.shape)
    peak = numpy.unravel_index(int(numpy.argmax(correlation)), correlation.shape)
    dy, dx = int(peak[0]), int(peak[1])
    if dy > height // 2:
        dy -= height
    if dx > width // 2:
        dx -= width
    return dx, dy


def tile_origins(shape):
    height = shape[0] - TEXT_HEIGHT
    for y in range(0, height - TILE_HEIGHT + 1, TILE_HEIGHT):
        for x in range(0, shape[1] - TILE_WIDTH + 1, TILE_WIDTH):
            yield y, x


def busiest_tiles(numpy, before, after, count=TILE_COUNT):
    scored = []
    for y, x in tile_origins(before.shape):
        tile_before = before[y : y + TILE_HEIGHT, x : x + TILE_WIDTH]
        tile_after = after[y : y + TILE_HEIGHT, x : x + TILE_WIDTH]
        gradient_y, gradient_x = numpy.gradient(tile_before)
        energy = float(numpy.hypot(gradient_x, gradient_y).std())
        dx, dy = tile_shift(numpy, tile_before, tile_after)
        scored.append({"energy": round(energy, 2), "dx": dx, "dy": dy, "y": y, "x": x})
    scored.sort(key=lambda row: -row["energy"])
    return scored[:count]


def luma_gap(numpy, before, after):
    return round(float(numpy.abs(after - before).mean()), 3)


def compare(numpy, before_path: Path, after_path: Path):
    from PIL import Image

    before = numpy.asarray(Image.open(before_path).convert("L"), dtype=numpy.float64)
    after = numpy.asarray(Image.open(after_path).convert("L"), dtype=numpy.float64)
    if before.shape != after.shape:
        raise base.StageError(
            "measure",
            f"{before_path.name} is {before.shape} and {after_path.name} is {after.shape}; the camera changed resolution mid-run",
        )
    report = {}
    crops_before = halves(numpy, before)
    crops_after = halves(numpy, after)
    for name, crop in crops_before.items():
        tiles = busiest_tiles(numpy, crop, crops_after[name])
        offsets = sorted(abs(tile["dx"]) for tile in tiles)
        middle = len(offsets) // 2
        median = offsets[middle] if len(offsets) % 2 else (offsets[middle - 1] + offsets[middle]) / 2
        report[name] = {
            "median_dx": median,
            "min_dx": offsets[0],
            "max_dx": offsets[-1],
            "moved_tiles": sum(1 for tile in tiles if abs(tile["dx"]) > MOVED_TILE_PX or abs(tile["dy"]) > MOVED_TILE_PX),
            "tiles": len(tiles),
            "mean_abs_luma": luma_gap(numpy, crop, crops_after[name]),
            "offsets": [{"dx": tile["dx"], "dy": tile["dy"], "energy": tile["energy"]} for tile in tiles],
        }
    return report


def save_crops(Image, source: Path, out_dir: Path, tag: str):
    image = Image.open(source).convert("RGB")
    width, height = image.size
    half = height // 2
    written = {}
    for name, box in {"top": (0, 0, width, half), "bottom": (0, half, width, height)}.items():
        path = out_dir / f"crop-{name}-{tag}.png"
        image.crop(box).save(path)
        written[name] = str(path)
    return written


def crop_stat(report, side, key):
    entry = report.get(side) or {}
    return float(entry.get(key, 0))


def call_json(http, method, path, timeout=120.0):
    status, body, raw = http.call(method, path, timeout=timeout)
    if status != 200 or body is None:
        detail = raw[:300].decode(errors="replace")
        raise base.StageError("http", f"{method} {path} returned {status}: {detail}")
    return body


def snapshot(http, out_dir: Path, tag: str):
    status, _, data = http.call("GET", f"/api/device/controls/snapshot?device={http.serial}", timeout=120.0)
    if status != 200 or not data:
        raise base.StageError("snapshot", f"snapshot {tag} returned {status}")
    path = out_dir / f"snapshot-{tag}.jpg"
    path.write_bytes(data)
    return path


def main(argv) -> int:
    parser = argparse.ArgumentParser(description="Prove the PTZ control path over the vendor cloud.")
    parser.add_argument("--repo", default="")
    parser.add_argument("--sn", default="")
    parser.add_argument("--pan", type=float, default=0.35)
    parser.add_argument("--hold-ms", type=int, default=600)
    parser.add_argument("--settle", type=float, default=1.6)
    parser.add_argument("--lan-target", default="192.168.0.100:34567")
    parser.add_argument("--rtsp-target", default="192.168.0.100:554")
    parser.add_argument("--env-file", default="~/re/proview-cctv/secrets/proview.env")
    parser.add_argument("--identity-file", default="~/re/proview-cctv/secrets/identity.env")
    parser.add_argument("--device-env-file", default="~/re/proview-cctv/secrets/device.env")
    parser.add_argument("--lock", default="~/re/proview-cctv/relay.lock")
    parser.add_argument("--lock-wait", type=float, default=180.0)
    parser.add_argument("--out", default="~/re/proview-cctv/acceptance-zha4281")
    parser.add_argument("--build-slot", default="~/re/proview-cctv/build-slot.sh")
    parser.add_argument("--bin", default="")
    parser.add_argument("--no-build", action="store_true")
    args = parser.parse_args(argv)
    args.mode = "cloud"
    args.fixture_dir = ""

    root = Path(args.repo).expanduser().resolve() if args.repo else base.repo_root()
    started = dt.datetime.now(dt.timezone.utc)
    prefix = base.git_text(root, "rev-parse", "--short=12", "HEAD") or "unknown"
    run_id = f"{prefix}-{started.strftime('%Y%m%dT%H%M%SZ')}"
    out_dir = Path(args.out).expanduser() / run_id
    out_dir.mkdir(parents=True, exist_ok=True)

    numpy, Image = load_views()

    report = {
        "schema": SCHEMA,
        "run_id": run_id,
        "started_at": now_stamp(started),
        "verdict": "fail",
        "failed_stage": None,
        "failure": None,
        "git": {
            "sha": base.git_text(root, "rev-parse", "HEAD"),
            "branch": base.git_text(root, "rev-parse", "--abbrev-ref", "HEAD"),
            "dirty": [line for line in base.git_text(root, "status", "--porcelain").splitlines() if line],
        },
        "host": {"name": os.uname().nodename, "kernel": os.uname().release, "numpy": numpy.__version__},
        "lan": {},
        "stages": [],
        "http": {},
        "move": {},
        "pixels": {},
        "verdict_numbers": {},
        "artifacts": {},
    }

    def stage(name, ok, detail, seconds=None):
        report["stages"].append({"name": name, "ok": ok, "detail": detail, "seconds": seconds})

    process = None
    log = None
    lock = None
    exit_code = 0
    try:
        clock = time.monotonic()
        for target in (args.lan_target, args.rtsp_target):
            guarded = base.guard_lan(target)
            report["lan"][target] = guarded
            stage("lan_guard", not guarded["reachable"], guarded["detail"], None)
            if guarded["reachable"]:
                raise base.StageError(
                    "lan_guard",
                    f"the camera answers at {target}, so a control call here would not prove the cloud path",
                    EXIT_LAN,
                )

        identity = base.read_env_file(args.identity_file)
        wanted = ("PROVIEW_APP_UUID", "PROVIEW_APP_KEY", "PROVIEW_APP_SECRET")
        missing = [key for key in wanted if not identity.get(key)]
        if missing:
            raise base.StageError(
                "identity",
                f"{args.identity_file} lacks {', '.join(missing)}; the build cannot sign the vendor envelope",
                base.EXIT_PREFLIGHT,
            )
        stage("identity", True, "3/3 keys from " + args.identity_file, round(time.monotonic() - clock, 3))

        clock = time.monotonic()
        binary, build_info = base.build_binary(root, args, identity)
        report["build"] = build_info
        stage(
            "build",
            True,
            f"{build_info['binary']} {'reused' if build_info['reused'] else 'built'} {build_info['mtime']}",
            round(time.monotonic() - clock, 3),
        )

        credentials = base.read_env_file(args.env_file)
        email = credentials.get("PROVIEW_EMAIL")
        password = credentials.get("PROVIEW_PASSWORD")
        bound_serial = args.sn.strip() or credentials.get("PROVIEW_DEVICE_SERIAL", "").strip()
        if not email or not password:
            raise base.StageError("credentials", f"{args.env_file} must define PROVIEW_EMAIL and PROVIEW_PASSWORD", base.EXIT_PREFLIGHT)
        stage("credentials", True, "sign-in material present", 0.0)

        clock = time.monotonic()
        lock = base.acquire_lock(args.lock, args.lock_wait)
        stage("relay_lock", True, f"exclusive lock on {Path(args.lock).expanduser()}", round(time.monotonic() - clock, 3))

        port = base.free_port()
        env = base.server_env(args, port, out_dir / "state")
        env.pop("PROVIEW_DEVICE_USER", None)
        env.pop("PROVIEW_DEVICE_PASS", None)
        if bound_serial:
            env["PROVIEW_DEVICE_SERIAL"] = bound_serial
        report["credentials"] = {
            "device_serial": bool(bound_serial),
            "device_password": "PROVIEW_DEVICE_PASS" in env,
        }
        clock = time.monotonic()
        process, log = base.start_server(binary, root, env, out_dir / "server.log")
        http = base.Http("127.0.0.1", port)
        ready = base.wait_ready(process, http, 90.0)
        stage("serve", True, f"127.0.0.1:{port} answered /api/health in {ready}s", round(time.monotonic() - clock, 3))

        clock = time.monotonic()
        status, body, raw = http.call("POST", "/api/login", {"email": email, "password": password}, timeout=180.0)
        message = body.get("message") if isinstance(body, dict) else raw[:200].decode(errors="replace")
        report["http"]["login"] = {"status": status, "message": message}
        if status != 200 or not (isinstance(body, dict) and body.get("ok")):
            raise base.StageError("login", f"POST /api/login returned {status}: {message}")
        stage("login", True, f"200 · {message}", round(time.monotonic() - clock, 3))

        clock = time.monotonic()
        resolved = call_json(http, "GET", "/api/config/mode", timeout=60.0)
        if resolved.get("mode") != "cloud" or resolved.get("source") != "cloud":
            raise base.StageError("mode", f"the server answered {json.dumps(resolved)}, so this run would not exercise the cloud provider")
        stage("mode", True, json.dumps(resolved), round(time.monotonic() - clock, 3))

        clock = time.monotonic()
        devices = call_json(http, "GET", "/api/devices", timeout=180.0)
        serials = [str(entry.get("id") or entry.get("uuid") or "") for entry in devices if isinstance(entry, dict)]
        serial = args.sn.strip() or bound_serial or (serials[0] if serials else "")
        if not serial:
            raise base.StageError("devices", "GET /api/devices listed no serial to control")
        http.serial = serial
        report["device"] = {"serial": serial, "listed": serials}
        stage("devices", True, f"{len(serials)} device(s), using {serial}", round(time.monotonic() - clock, 3))

        clock = time.monotonic()
        controls = call_json(http, "GET", f"/api/device/controls?device={serial}", timeout=120.0)
        rows = controls.get("capabilities") or []
        report["controls"] = rows
        moved_row = next((row for row in rows if row.get("feature") == "ptz move" and row.get("lens") == "bottom"), None)
        top_row = next((row for row in rows if row.get("feature") == "ptz move" and row.get("lens") == "top"), None)
        if controls.get("dvrip") is not True:
            raise base.StageError("controls", "the report says the DVRIP backend is not configured in cloud mode")
        if not moved_row or moved_row.get("state") != "supported":
            raise base.StageError("controls", f"the report does not offer a ptz move on the lower lens: {json.dumps(moved_row)}")
        if not top_row or top_row.get("state") != "blocked":
            raise base.StageError("controls", f"the report does not refuse a move on the upper lens: {json.dumps(top_row)}")
        stage("controls", True, f"ptz move on the lower lens via {moved_row.get('backend')}, upper lens refused", round(time.monotonic() - clock, 3))

        clock = time.monotonic()
        presets = call_json(http, "GET", f"/api/device/controls/presets?device={serial}", timeout=120.0)
        report["presets"] = presets
        if not presets:
            raise base.StageError(
                "presets",
                "the device lists no preset, so GotoPreset cannot return the lens to where it started; set one in the vendor app first",
            )
        preset = presets[0]
        stage("presets", True, f"{len(presets)} preset(s), using {preset.get('name')} ({preset.get('token')})", round(time.monotonic() - clock, 3))

        clock = time.monotonic()
        call_json(http, "POST", f"/api/device/controls/preset/goto?token={preset['token']}&device={serial}", timeout=180.0)
        time.sleep(max(1.5, args.settle))
        before_one = snapshot(http, out_dir, "before")
        time.sleep(0.8)
        before_two = snapshot(http, out_dir, "before-second")
        stage("baseline", True, f"parked at preset {preset['token']} and read two frames", round(time.monotonic() - clock, 3))

        clock = time.monotonic()
        moved = call_json(
            http,
            "POST",
            f"/api/device/controls/ptz?lens=bottom&pan={args.pan}&tilt=0&hold_ms={args.hold_ms}&device={serial}",
            timeout=180.0,
        )
        report["move"] = moved
        time.sleep(max(1.5, args.settle))
        after_one = snapshot(http, out_dir, "after")
        time.sleep(0.8)
        after_two = snapshot(http, out_dir, "after-second")
        stage("move", True, f"pan {args.pan} for {args.hold_ms} ms over {moved.get('backend')}", round(time.monotonic() - clock, 3))

        clock = time.monotonic()
        call_json(http, "POST", f"/api/device/controls/preset/goto?token={preset['token']}&device={serial}", timeout=180.0)
        time.sleep(max(1.5, args.settle))
        restored_one = snapshot(http, out_dir, "restored")
        time.sleep(0.8)
        restored_two = snapshot(http, out_dir, "restored-second")
        stage("restore", True, f"GotoPreset {preset['token']} sent and two frames read", round(time.monotonic() - clock, 3))

        clock = time.monotonic()
        noise = compare(numpy, before_one, before_two)
        moved_pixels = compare(numpy, before_two, after_two)
        restored = compare(numpy, before_two, restored_two)
        report["pixels"] = {"noise": noise, "moved": moved_pixels, "restored": restored}
        report["artifacts"]["crops"] = {
            "before": save_crops(Image, before_two, out_dir, "before"),
            "after": save_crops(Image, after_two, out_dir, "after"),
            "restored": save_crops(Image, restored_two, out_dir, "restored"),
        }
        report["artifacts"]["frames"] = [
            str(path)
            for path in (before_one, before_two, after_one, after_two, restored_one, restored_two)
        ]
        for crop in ("top", "bottom"):
            summary = (
                f"{crop}: median shift {moved_pixels[crop]['median_dx']:.1f} px "
                f"({moved_pixels[crop]['min_dx']}-{moved_pixels[crop]['max_dx']} px over "
                f"{moved_pixels[crop]['tiles']} busiest tiles, noise {noise[crop]['median_dx']:.0f} px), "
                f"{moved_pixels[crop]['moved_tiles']}/{moved_pixels[crop]['tiles']} tiles moved; "
                f"restored {restored[crop]['median_dx']:.1f} px, "
                f"{restored[crop]['moved_tiles']}/{restored[crop]['tiles']} tiles moved; "
                f"luma {moved_pixels[crop]['mean_abs_luma']} (noise {noise[crop]['mean_abs_luma']})"
            )
            stage("pixels-" + crop, True, summary, round(time.monotonic() - clock, 3))

        bottom_shift = crop_stat(moved_pixels, "bottom", "median_dx")
        top_shift = crop_stat(moved_pixels, "top", "median_dx")
        back_shift = crop_stat(restored, "bottom", "median_dx")
        noise_floor = max(
            crop_stat(noise, "bottom", "median_dx"),
            crop_stat(noise, "top", "median_dx"),
        )
        bottom_tiles = int(crop_stat(moved_pixels, "bottom", "moved_tiles"))
        bottom_total = int(crop_stat(moved_pixels, "bottom", "tiles"))
        top_tiles = int(crop_stat(moved_pixels, "top", "moved_tiles"))
        back_tiles = int(crop_stat(restored, "bottom", "moved_tiles"))
        report["verdict_numbers"] = {
            "bottom_moved_px": bottom_shift,
            "top_moved_px": top_shift,
            "restored_px": back_shift,
            "noise_px": noise_floor,
            "bottom_tiles": f"{bottom_tiles}/{bottom_total}",
            "top_tiles": f"{top_tiles}/{bottom_total}",
            "restored_tiles": f"{back_tiles}/{bottom_total}",
            "move_min_px": MOVE_MIN_PX,
            "still_max_px": STILL_MAX_PX,
            "bottom_luma": moved_pixels["bottom"]["mean_abs_luma"],
            "bottom_luma_noise": noise["bottom"]["mean_abs_luma"],
        }
        failures = []
        if bottom_shift < MOVE_MIN_PX:
            failures.append(f"the lower lens moved {bottom_shift:.1f} px, under the {MOVE_MIN_PX} px floor")
        if bottom_tiles < 5:
            failures.append(f"only {bottom_tiles} of {bottom_total} busy tiles on the lower lens moved")
        if top_shift > max(STILL_MAX_PX, noise_floor + 1) or top_tiles > 1:
            failures.append(f"the upper lens moved ({top_shift:.1f} px over {top_tiles} tiles), over the still ceiling")
        if back_shift > max(STILL_MAX_PX + 2, noise_floor + 1):
            failures.append(f"GotoPreset left the lower lens {back_shift:.1f} px from where it started")
        if failures:
            raise base.StageError("verdict", "; ".join(failures))
        stage("verdict", True, "the lower lens moved, the upper lens stayed, GotoPreset put it back", 0.0)
        report["verdict"] = "pass"
    except base.StageError as error:
        report["failed_stage"] = error.stage
        report["failure"] = error.detail
        report["verdict"] = "fail"
        exit_code = error.code
        stage(error.stage, False, error.detail, None)
    finally:
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=15)
            report["server"] = {"exit": process.returncode, "log": str(out_dir / "server.log")}
        if log is not None:
            log.close()
        if lock is not None:
            lock.close()
        report["finished_at"] = now_stamp(dt.datetime.now(dt.timezone.utc))
        (out_dir / "report.json").write_text(json.dumps(report, indent=2))
        (out_dir / "report.md").write_text(render(report))
        print(f"verdict: {report['verdict']}")
        if report["failed_stage"]:
            print(f"stage: {report['failed_stage']} · {report['failure']}")
        numbers = report.get("verdict_numbers") or {}
        if numbers:
            print(
                "bottom {bottom_moved_px:.1f} px ({bottom_tiles} tiles), top {top_moved_px:.1f} px "
                "({top_tiles} tiles), restored {restored_px:.1f} px, noise {noise_px:.1f} px".format(**numbers)
            )
        print(f"report: {out_dir / 'report.md'}")
    return exit_code


def render(report: dict) -> str:
    lines = [f"# Cloud PTZ acceptance · {report['run_id']}", "", f"Verdict: **{report['verdict']}**"]
    if report["failed_stage"]:
        lines.append(f"Stopped at stage `{report['failed_stage']}`: {report['failure']}")
    lines += ["", "| stage | ok | seconds | detail |", "|---|---|---|---|"]
    for item in report["stages"]:
        detail = str(item["detail"]).replace("|", "/")
        lines.append(f"| {item['name']} | {'yes' if item['ok'] else 'no'} | {item.get('seconds', '')} | {detail} |")
    pixels = report.get("pixels") or {}
    if pixels:
        lines += [
            "",
            "## Pixels",
            "",
            "Each crop is cut into 60x80 tiles, the eight with the most edge energy are measured by phase",
            "correlation, and the table reports the median absolute horizontal shift. A near wall gives the",
            "lower lens parallax, so the range is as much a result as the median.",
            "",
            "| crop | median move | range | tiles moved | median restored | restored tiles | median noise | luma | luma noise |",
            "|---|---|---|---|---|---|---|---|---|",
        ]
        for crop in ("top", "bottom"):
            moved = pixels["moved"][crop]
            restored = pixels["restored"][crop]
            noise = pixels["noise"][crop]
            lines.append(
                f"| {crop} | {moved['median_dx']:.1f} px | {moved['min_dx']}-{moved['max_dx']} px | "
                f"{moved['moved_tiles']}/{moved['tiles']} | {restored['median_dx']:.1f} px | "
                f"{restored['moved_tiles']}/{restored['tiles']} | {noise['median_dx']:.1f} px | "
                f"{moved['mean_abs_luma']} | {noise['mean_abs_luma']} |"
            )
    controls = report.get("controls") or []
    if controls:
        lines += ["", "## Capability report", "", "| lens | feature | state | backend | evidence |", "|---|---|---|---|---|"]
        for row in controls:
            evidence = str(row.get("evidence", "")).replace("|", "/")
            lines.append(f"| {row.get('lens')} | {row.get('feature')} | {row.get('state')} | {row.get('backend')} | {evidence} |")
    artifacts = report.get("artifacts") or {}
    if artifacts.get("frames"):
        lines += ["", "## Artifacts", ""]
        for path in artifacts["frames"]:
            lines.append("- `" + path + "`")
    lines += ["", f"git: {report['git']['sha']} ({report['git']['branch']})"]
    return "\n".join(lines) + "\n"


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
