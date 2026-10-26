#!/usr/bin/env bash
set -euo pipefail

binary=${1:-target/release/proview-server}
port=${PORT:-8791}
root=$(cd "$(dirname "$0")/.." && pwd)

if [ ! -x "$binary" ]; then
  echo "build first: cargo build --release -p proview-server" >&2
  exit 1
fi
if [ ! -f "$root/fixtures/fixture-dev-0001_ch1_main.h264" ]; then
  echo "generate a clip first: ffmpeg -f lavfi -i testsrc=size=640x360:rate=25:duration=3 -c:v libx264 -profile:v baseline -pix_fmt yuv420p -f h264 fixtures/fixture-dev-0001_ch1_main.h264" >&2
  exit 1
fi

ROCKET_SECRET_KEY=$(openssl rand -base64 32) ROCKET_PORT="$port" ROCKET_ADDRESS=127.0.0.1 \
  "$binary" > /tmp/proview-measure.log 2>&1 &
pid=$!
trap 'kill $pid 2>/dev/null || true' EXIT

for _ in $(seq 1 40); do
  sleep 0.25
  if curl -sf -m 1 "http://127.0.0.1:$port/api/health" > /dev/null; then break; fi
done

curl -s -c /tmp/proview-measure.cookies -X POST "http://127.0.0.1:$port/api/login" \
  -H 'content-type: application/json' \
  -d '{"email":"fixture@example.invalid","password":"fixture-password"}' > /dev/null

measure() {
  local label="$1"
  local runs="$2"
  local total_ttfb=0
  local total_full=0
  local bytes=0
  for _ in $(seq 1 "$runs"); do
    read -r ttfb full size < <(
      curl -s -b /tmp/proview-measure.cookies -o /dev/null \
        -w '%{time_starttransfer} %{time_total} %{size_download}\n' \
        "http://127.0.0.1:$port/media/fixture-dev-0001/1/main.mp4"
    )
    total_ttfb=$(awk -v a="$total_ttfb" -v b="$ttfb" 'BEGIN{printf "%.4f", a+b}')
    total_full=$(awk -v a="$total_full" -v b="$full" 'BEGIN{printf "%.4f", a+b}')
    bytes="$size"
  done
  awk -v label="$label" -v ttfb="$total_ttfb" -v full="$total_full" -v n="$runs" -v bytes="$bytes" \
    'BEGIN{printf "%-22s ttfb=%.1f ms  total=%.1f ms  bytes=%d\n", label, ttfb*1000/n, full*1000/n, bytes}'
}

rss_kb=$(awk '/VmRSS/{print $2}' /proc/$pid/status)
cpu_before=$(awk '{print $14+$15}' /proc/$pid/stat)

measure "clip cold" 1
measure "clip warm x10" 10

cpu_after=$(awk '{print $14+$15}' /proc/$pid/stat)
rss_after_kb=$(awk '/VmRSS/{print $2}' /proc/$pid/status)
awk -v a="$cpu_before" -v b="$cpu_after" -v rss="$rss_after_kb" \
  'BEGIN{printf "%-22s %.0f ms cpu total  rss=%.1f MiB\n", "process", (b-a)*10, rss/1024}'
