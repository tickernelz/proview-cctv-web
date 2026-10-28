#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
prefix=${IMAGE:-proview-webkitgtk}
for v in 22.04 24.04; do
  docker build -q --memory 4g --build-arg UBUNTU="$v" -f Dockerfile.ubuntu -t "$prefix:ubuntu-$v" .
done
docker build -q --memory 4g --build-arg FEDORA=44 -f Dockerfile.fedora -t "$prefix:fedora-44" .
docker images "$prefix" --format '{{.Repository}}:{{.Tag}}'
