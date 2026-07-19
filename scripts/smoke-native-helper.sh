#!/usr/bin/env bash

set -euo pipefail

if [[ "$#" -ne 3 ]]; then
  echo "usage: $0 <native-helper> <plantuml-fixture> <new-smoke-root>" >&2
  exit 2
fi

helper="$1"
fixture="$2"
smoke_root="${3//\\//}"
smoke_support_root="${smoke_root}.support"

if [[ ! -f "${helper}" ]]; then
  echo "native helper does not exist: ${helper}" >&2
  exit 2
fi
if [[ ! -f "${fixture}" ]]; then
  echo "PlantUML smoke fixture does not exist: ${fixture}" >&2
  exit 2
fi
if [[ -e "${smoke_root}" ]]; then
  echo "smoke root must not already exist: ${smoke_root}" >&2
  exit 2
fi
if [[ -e "${smoke_support_root}" ]]; then
  echo "smoke support root must not already exist: ${smoke_support_root}" >&2
  exit 2
fi

mkdir -p \
  "${smoke_root}" \
  "${smoke_support_root}/home" \
  "${smoke_support_root}/cache" \
  "${smoke_support_root}/state" \
  "${smoke_support_root}/local-app-data"
cp "${fixture}" "${smoke_root}/diagram.puml"

# Keep the managed cache and external transaction state clean, private, and on
# the same volume as the smoke output on every runner (especially Windows).
export HOME="${smoke_support_root}/home"
export XDG_CACHE_HOME="${smoke_support_root}/cache"
export XDG_STATE_HOME="${smoke_support_root}/state"
export LOCALAPPDATA="${smoke_support_root}/local-app-data"

# Give the first managed install and PlantUML JVM startup the export path's
# 120-second render budget. The separate interactive diagnostic path keeps its
# intentionally shorter budget and is covered by the CLI/LSP contract tests.
"${helper}" --root "${smoke_root}" --json export \
  --format svg \
  --out-dir out \
  diagram.puml
test -s "${smoke_root}/out/diagram.svg"

"${helper}" --root "${smoke_root}" --json health

for format in png pdf; do
  "${helper}" --root "${smoke_root}" --json export \
    --format "${format}" \
    --out-dir out \
    diagram.puml
  test -s "${smoke_root}/out/diagram.${format}"
done

output_count="$(find "${smoke_root}/out" -type f | wc -l | tr -d '[:space:]')"
test "${output_count}" = "3"
