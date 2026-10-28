#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
runs=${RUNS:-5}
engines=${ENGINES:-chrome}
out=${OUT:-results-first-frame.jsonl.tmp}
: > "$out"
for engine in $engines; do
  node first-frame.mjs "$engine" ../../web "$runs" 0 | tee -a "$out"
  for d in vanilla-ts/dist vue-ts/dist react-ts/dist solid-ts/dist svelte-ts/dist sveltekit/build astro/dist leptos/dist yew/dist dioxus/dist; do
    node first-frame.mjs "$engine" "$d" "$runs" 9 | tee -a "$out"
  done
done
