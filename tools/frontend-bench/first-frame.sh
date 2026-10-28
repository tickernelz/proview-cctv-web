#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
runs=${RUNS:-5}
engines=${ENGINES:-chrome webkitgtk}
for engine in $engines; do
  node first-frame.mjs "$engine" ../../web "$runs" 0
  for d in vanilla-ts/dist vue-ts/dist react-ts/dist solid-ts/dist svelte-ts/dist sveltekit/build astro/dist leptos/dist yew/dist dioxus/dist; do
    node first-frame.mjs "$engine" "$d" "$runs" 9
  done
done
