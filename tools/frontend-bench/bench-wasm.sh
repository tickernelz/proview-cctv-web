#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
options=${OPTIONS:-leptos yew dioxus}
out=${OUT:-results-wasm.tsv}
target_root=${TARGET_ROOT:-$(mktemp -d)}
now() { date +%s.%N; }
since() { awk -v a="$1" -v b="$(now)" 'BEGIN { printf "%.1f", b - a }'; }
gz() { local t=0 f; for f in "$@"; do t=$((t + $(gzip -9c "$f" | wc -c))); done; echo "$t"; }
raw() { local t=0 f; for f in "$@"; do t=$((t + $(stat -c%s "$f"))); done; echo "$t"; }
printf 'option\tcold_build_s\twarm_build_s\twasm_raw\twasm_gz\tjs_gz\tcss_gz\thtml_gz\ttotal_gz\tcrates\n' > "$out"
for d in $options; do
  export CARGO_TARGET_DIR="$target_root/$d"
  rm -rf "$d/dist" "$CARGO_TARGET_DIR"
  t=$(now)
  (cd "$d" && ${SLOT:-} trunk build --release > "../build-$d.log" 2>&1) || { printf '%s\tfail\n' "$d" >> "$out"; continue; }
  cold=$(since "$t")
  rm -rf "$d/dist"
  touch "$d/src/main.rs"
  t=$(now)
  (cd "$d" && ${SLOT:-} trunk build --release >> "../build-$d.log" 2>&1)
  warm=$(since "$t")
  mapfile -t wasm < <(find "$d/dist" -type f -name '*.wasm')
  mapfile -t js < <(find "$d/dist" -type f -name '*.js')
  mapfile -t cs < <(find "$d/dist" -type f -name '*.css')
  mapfile -t ht < <(find "$d/dist" -type f -name '*.html')
  wg=$(gz "${wasm[@]}"); jg=$(gz "${js[@]}"); cg=$(gz "${cs[@]}"); hg=$(gz "${ht[@]}")
  crates=$(grep -c '^name = ' "$d/Cargo.lock")
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$d" "$cold" "$warm" "$(raw "${wasm[@]}")" "$wg" "$jg" "$cg" "$hg" $((wg + jg + cg + hg)) "$crates" >> "$out"
done
column -t -s $'\t' "$out"
