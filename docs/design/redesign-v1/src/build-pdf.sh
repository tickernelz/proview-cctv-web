#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
PORT=${PORT:-18944}
CHROME=${CHROME:-google-chrome}
python3 -m http.server "$PORT" --bind 127.0.0.1 >/dev/null 2>&1 &
SERVER=$!
trap 'kill $SERVER 2>/dev/null || true' EXIT
sleep 1
"$CHROME" --headless=new --disable-gpu --no-pdf-header-footer --virtual-time-budget=10000 \
  --print-to-pdf=proview-redesign-v1.pdf "http://127.0.0.1:$PORT/src/doc.html" >/dev/null 2>&1
pdfinfo proview-redesign-v1.pdf | grep -E '^(Pages|Page size|File size)'
