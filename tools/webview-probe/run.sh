#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
engine=$1
prefix=${IMAGE:-proview-webkitgtk}
if [ "$engine" != chrome ]; then
  docker run --rm --network none --memory 2g --memory-swap 2g -v "$PWD:/p:ro" "$prefix:$engine" sh /p/inside.sh /p probe.html 2> /dev/null
  exit
fi
port=${PORT:-18433}
out=$(mktemp)
profile=$(mktemp -d)
node serve.mjs "$port" "$out" &
server=$!
sleep 0.5
google-chrome --headless=new --no-sandbox --user-data-dir="$profile" "http://127.0.0.1:$port/" > /dev/null 2>&1 &
browser=$!
wait "$server" || true
kill "$browser" 2> /dev/null || true
cat "$out"
rm -rf "$out" "$profile"
