#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
PORT=${PORT:-18943}
OUT=shots
CHROME=${CHROME:-google-chrome}
SCALE=${SCALE:-1.5}
mkdir -p "$OUT"
python3 -m http.server "$PORT" --bind 127.0.0.1 >/dev/null 2>&1 &
SERVER=$!
trap 'kill $SERVER 2>/dev/null || true' EXIT
sleep 1

grab() {
  local url=$1 w=$2 h=$3 name=$4
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor="$SCALE" \
    --window-size="$w,$h" --virtual-time-budget=6000 \
    --screenshot="$OUT/$name.png" "http://127.0.0.1:$PORT/$url" >/dev/null 2>&1
  ffmpeg -y -loglevel error -i "$OUT/$name.png" -q:v 3 "$OUT/$name.jpg"
  rm -f "$OUT/$name.png"
  printf '%-30s %s\n' "$name" "$(du -h "$OUT/$name.jpg" | cut -f1)"
}

both() {
  local screen=$1 w=${2:-1440} h=${3:-900} suffix=${4:-}
  grab "src/screen.html?screen=$screen&theme=dark" "$w" "$h" "$screen$suffix-dark"
  grab "src/screen.html?screen=$screen&theme=light" "$w" "$h" "$screen$suffix-light"
}

for s in signin signin-error signin-expired devices-empty live-single live live-switcher \
         lens-menu lenses ptz fullscreen talk record grid grid2 grid-fail \
         boot live-loading live-fail live-stalled live-offline nonetwork \
         settings-playback settings-device settings-config; do
  both "$s"
done
for s in signin live grid settings-playback ptz; do
  both "$s" 1280 800 -1280
done
grab src/dir-a.html 1440 900 dir-a
grab src/dir-c.html 1440 900 dir-c
