#!/usr/bin/env bash

set -euo pipefail

if [[ "$#" -ne 3 ]]; then
  echo "usage: $0 <native-helper> <plantuml-fixture> <new-smoke-root>" >&2
  exit 2
fi

helper="$1"
fixture="$2"
smoke_root="${3//\\//}"

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

mkdir -p "${smoke_root}"
cp "${fixture}" "${smoke_root}/diagram.puml"

"${helper}" --root "${smoke_root}" --json check diagram.puml
"${helper}" --root "${smoke_root}" --json health

for format in svg png pdf; do
  "${helper}" --root "${smoke_root}" --json export \
    --format "${format}" \
    --out-dir out \
    diagram.puml
  test -s "${smoke_root}/out/diagram.${format}"
done

output_count="$(find "${smoke_root}/out" -type f | wc -l | tr -d '[:space:]')"
test "${output_count}" = "3"
