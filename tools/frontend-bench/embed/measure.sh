#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
runs=${RUNS:-10}
port=${PORT:-18440}
rm -rf gz
mkdir -p gz/fonts
(cd ../../../web && for f in *.html *.js *.css fonts/*; do gzip -9c "$f" > "../tools/frontend-bench/embed/gz/$f.gz"; done)
printf 'mode\tbinary_bytes\tbuild_s\tstart_to_index_ms_median\tstart_to_index_ms_all\tindex_wire_bytes\n'
for mode in none per-route embedded-dir embedded-gz; do
  features=""
  [ "$mode" = none ] || features="--features $mode"
  t=$(date +%s.%N)
  ${SLOT:-} cargo build --release --quiet $features
  build=$(awk -v a="$t" -v b="$(date +%s.%N)" 'BEGIN { printf "%.1f", b - a }')
  bin="${CARGO_TARGET_DIR:-target}/release/bench-embed"
  size=$(stat -c%s "$bin")
  times=()
  for _ in $(seq "$runs"); do
    s=$(date +%s%N)
    ROCKET_PORT=$port ROCKET_LOG_LEVEL=off "$bin" > /dev/null 2>&1 &
    pid=$!
    until curl -sf -o /dev/null "http://127.0.0.1:$port/"; do sleep 0.002; done
    times+=($((($(date +%s%N) - s) / 1000000)))
    kill "$pid"
    wait "$pid" 2> /dev/null || true
  done
  ROCKET_PORT=$port ROCKET_LOG_LEVEL=off "$bin" > /dev/null 2>&1 &
  pid=$!
  until curl -sf -o /dev/null "http://127.0.0.1:$port/"; do sleep 0.002; done
  wire=$(curl -s "http://127.0.0.1:$port/" | wc -c)
  kill "$pid"
  wait "$pid" 2> /dev/null || true
  median=$(printf '%s\n' "${times[@]}" | sort -n | awk '{ a[NR] = $1 } END { print a[int((NR + 1) / 2)] }')
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$mode" "$size" "$build" "$median" "$(IFS=,; echo "${times[*]}")" "$wire"
done
