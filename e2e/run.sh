#!/usr/bin/env bash
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
version=$(node -p "require('$here/node_modules/@e2e-dev/web/node_modules/playwright-core/package.json').version" 2>/dev/null ||
  node -p "require('$here/node_modules/playwright-core/package.json').version")

exec docker run --rm --init --ipc host --network host \
  --memory "${E2E_DOCKER_MEMORY:-4g}" --memory-swap "${E2E_DOCKER_MEMORY:-4g}" \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp -e CI -e E2E_WORKERS -e E2E_MODEL -e E2E_MODEL_BASE_URL -e E2E_MODEL_API_KEY \
  -e E2E_TELEMETRY_DISABLED=1 -e DO_NOT_TRACK=1 -e E2E_UPDATE_CHECK_DISABLED=1 \
  -v "$root:$root" -w "$here" \
  "mcr.microsoft.com/playwright:v${version}-noble" \
  npx --no-install e2e run "$@"
