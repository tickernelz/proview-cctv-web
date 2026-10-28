#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
engine=$1
port=${PORT:-18433}
out=$(mktemp)
node serve.mjs "$port" "$out" &
server=$!
sleep 0.5
case "$engine" in
  webkitgtk)
    systemd-run --user --scope -q --collect -u "zha4133-probe-$engine" env GDK_BACKEND=x11 xvfb-run -a /usr/libexec/webkit2gtk-4.1/MiniBrowser "http://127.0.0.1:$port/" > /dev/null 2>&1 &
    ;;
  ubuntu-*)
    docker run --rm --name "zha4133-probe-$engine" --memory 4g --memory-swap 4g --network host -e WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1 -v "$PWD:/p:ro" "zha4133-webkit:${engine#ubuntu-}" timeout 60 xvfb-run -a python3 -u /p/gtk_probe.py "http://127.0.0.1:$port/" >&2 &
    ;;
  chrome)
    systemd-run --user --scope -q --collect -u "zha4133-probe-$engine" google-chrome --headless=new --no-sandbox --user-data-dir="$(mktemp -d)" "http://127.0.0.1:$port/" > /dev/null 2>&1 &
    ;;
esac
wait "$server" || true
systemctl --user stop "zha4133-probe-$engine.scope" 2> /dev/null || true
docker rm -f "zha4133-probe-$engine" > /dev/null 2>&1 || true
cat "$out"
rm -f "$out"
