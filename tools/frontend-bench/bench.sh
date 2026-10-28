#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
runs=${RUNS:-3}
options=${OPTIONS:-vanilla-ts vue-ts react-ts solid-ts svelte-ts sveltekit solidstart astro}
out=${OUT:-results.tsv}
now() { date +%s.%N; }
since() { awk -v a="$1" -v b="$(now)" 'BEGIN { printf "%.1f", b - a }'; }
outdir() {
  case "$1" in
    sveltekit) echo build ;;
    solidstart) echo .output/public ;;
    *) echo dist ;;
  esac
}
gz() { local t=0 f; for f in "$@"; do t=$((t + $(gzip -9c "$f" | wc -c))); done; echo "$t"; }
raw() { local t=0 f; for f in "$@"; do t=$((t + $(stat -c%s "$f"))); done; echo "$t"; }
printf 'option\tnpm_ci_s\tbuild_s\tjs_raw\tjs_gz\tcss_gz\thtml_gz\ttotal_gz\tjs_files\n' > "$out"
for d in $options; do
  rm -rf "$d/node_modules"
  cache=$(mktemp -d)
  t=$(now)
  (cd "$d" && NODE_ENV=development nice -n 10 npm ci --include=dev --no-audit --no-fund --cache "$cache" > /dev/null 2>&1)
  ci=$(since "$t")
  rm -rf "$cache"
  o=$(outdir "$d")
  builds=()
  for _ in $(seq "$runs"); do
    rm -rf "$d/$o" "$d/.svelte-kit" "$d/.astro" "$d/node_modules/.vite" "$d/node_modules/.astro"
    t=$(now)
    if (cd "$d" && NODE_ENV=production nice -n 10 npm run build > "../build-$d.log" 2>&1); then
      builds+=("$(since "$t")")
    else
      builds+=(fail)
    fi
  done
  mapfile -t js < <(find "$d/$o" -type f -name '*.js')
  mapfile -t cs < <(find "$d/$o" -type f -name '*.css')
  mapfile -t ht < <(find "$d/$o" -type f -name '*.html')
  jg=$(gz "${js[@]}"); cg=$(gz "${cs[@]}"); hg=$(gz "${ht[@]}")
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$d" "$ci" "$(IFS=/; echo "${builds[*]}")" "$(raw "${js[@]}")" "$jg" "$cg" "$hg" $((jg + cg + hg)) "${#js[@]}" >> "$out"
done
column -t -s $'\t' "$out"
