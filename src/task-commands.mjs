import {
  PLANTUML_DEFAULT_SHA256,
  PLANTUML_DEFAULT_VERSION,
  plantumlPdfDependencies,
} from "./renderer.mjs";

export const PLANTUML_TASK_HELPER_VERSION = 1;
export const PLANTUML_TASK_PLANTUML_VERSION = PLANTUML_DEFAULT_VERSION;

const shell = {
  with_arguments: {
    program: "/bin/sh",
    args: ["-c"],
  },
};

export function buildPlantUmlTasks() {
  return [
    currentFileTask({
      label: "PlantUML: export current file to PNG",
      format: "png",
      tags: ["plantuml-export-png"],
    }),
    currentFileTask({
      label: "PlantUML: export current file to PNG and open",
      format: "png",
      tags: ["plantuml-export-png-open"],
      afterExport: openCurrentPngSnippet(),
    }),
    currentFileTask({
      label: "PlantUML: export current file to SVG",
      format: "svg",
      tags: ["plantuml-export-svg"],
    }),
    currentFileTask({
      label: "PlantUML: export current file to PDF",
      format: "pdf",
      tags: ["plantuml-export-pdf"],
    }),
    healthCheckTask(),
    workspaceTask({
      label: "PlantUML: export workspace to PNG",
      format: "png",
      tags: ["plantuml-export-workspace-png"],
    }),
    workspaceTask({
      label: "PlantUML: export workspace to SVG",
      format: "svg",
      tags: ["plantuml-export-workspace-svg"],
    }),
  ];
}

export function renderPlantUmlTasksJson() {
  return `${JSON.stringify(buildPlantUmlTasks(), null, 2)}\n`;
}

function currentFileTask({ afterExport = "", format, label, tags }) {
  return task({
    command: [
      taskPrelude(),
      `run_plantuml -t${format} -o "$out" "$ZED_FILE"`,
      afterExport,
    ].filter(Boolean).join("; "),
    hide: "on_success",
    label,
    save: "current",
    tags,
  });
}

function workspaceTask({ format, label, tags }) {
  return task({
    command: [
      taskPrelude(),
      findPumlSnippet(),
      `if [ -z "$(find_puml -print -quit)" ]; then echo "No PlantUML files found under $ZED_WORKTREE_ROOT" >&2; exit 1; fi`,
      workspaceRenderSnippet(format),
    ].join("; "),
    hide: "on_success",
    label,
    save: "all",
    tags,
  });
}

function healthCheckTask() {
  return task({
    command: [
      taskHeader(),
      resolveOutDirSnippet(),
      checksumSnippet(),
      ensurePlantUmlJarSnippet(),
      ensurePdfLibsSnippet(),
      `echo "PlantUML renderer health check"`,
      `if [ -n "\${PLANTUML_ZED_PLANTUML_BIN:-}" ]; then echo "PLANTUML_ZED_PLANTUML_BIN=$PLANTUML_ZED_PLANTUML_BIN"; fi`,
      `if [ -n "\${PLANTUML_ZED_PLANTUML_JAR:-}" ]; then echo "PLANTUML_ZED_PLANTUML_JAR=$PLANTUML_ZED_PLANTUML_JAR"; fi`,
      `if command -v plantuml >/dev/null 2>&1; then echo "plantuml command:"; plantuml -version || true; else echo "plantuml command: not found"; fi`,
      `if [ -n "\${PLANTUML_JAR:-}" ] && [ -f "$PLANTUML_JAR" ]; then echo "PLANTUML_JAR=$PLANTUML_JAR"; fi`,
      `jar="$(ensure_plantuml_jar)"`,
      `echo "cached jar: $jar"`,
      `echo "PDF sidecar libraries: downloaded on first cached-jar PDF export"`,
      `"$java_bin" -version`,
      `"$java_bin" -jar "$jar" -version`,
      `if command -v "$dot_bin" >/dev/null 2>&1; then echo "Graphviz:"; "$dot_bin" -V; else echo "Graphviz: dot command not found"; fi`,
    ].join("; "),
    label: "PlantUML: renderer health check",
    save: "current",
    tags: ["plantuml-health-check"],
  });
}

function task({ command, hide, label, save, tags }) {
  return {
    label,
    command,
    save,
    ...(hide ? { hide } : {}),
    tags,
    shell,
  };
}

function taskPrelude() {
  return [
    taskHeader(),
    resolveOutDirSnippet(),
    `mkdir -p "$out"`,
    checksumSnippet(),
    ensurePlantUmlJarSnippet(),
    ensurePdfLibsSnippet(),
    runPlantUmlSnippet(),
  ].join("; ");
}

function taskHeader() {
  return [
    `set -eu`,
    `PLANTUML_TASK_HELPER_VERSION=${PLANTUML_TASK_HELPER_VERSION}`,
    `PLANTUML_DEFAULT_VERSION="${PLANTUML_TASK_PLANTUML_VERSION}"`,
    `PLANTUML_DEFAULT_SHA256="${PLANTUML_DEFAULT_SHA256}"`,
    `PLANTUML_VERSION="\${PLANTUML_ZED_PLANTUML_VERSION:-$PLANTUML_DEFAULT_VERSION}"`,
    `PLANTUML_SHA256="\${PLANTUML_ZED_PLANTUML_SHA256:-}"`,
    `if [ -z "$PLANTUML_SHA256" ] && [ "$PLANTUML_VERSION" = "$PLANTUML_DEFAULT_VERSION" ]; then PLANTUML_SHA256="$PLANTUML_DEFAULT_SHA256"; fi`,
    `java_bin="\${PLANTUML_ZED_JAVA:-java}"`,
    `dot_bin="\${PLANTUML_ZED_GRAPHVIZ_DOT:-dot}"`,
  ].join("; ");
}

function resolveOutDirSnippet() {
  return `task_out="\${PLANTUML_ZED_OUTPUT_DIR:-out/plantuml}"; case "$task_out" in /*) out="$task_out" ;; *) out="$ZED_WORKTREE_ROOT/$task_out" ;; esac`;
}

function ensurePlantUmlJarSnippet() {
  return `ensure_plantuml_jar() { if ! command -v "$java_bin" >/dev/null 2>&1; then echo "Java is required for automatic PlantUML rendering. Install Java or install the 'plantuml' command." >&2; exit 1; fi; cache="\${XDG_CACHE_HOME:-$HOME/.cache}/zed-plantuml"; if [ "$(uname)" = "Darwin" ] && [ -z "\${XDG_CACHE_HOME:-}" ]; then cache="$HOME/Library/Caches/zed-plantuml"; fi; jar="$cache/plantuml-$PLANTUML_VERSION.jar"; if [ -s "$jar" ] && [ -n "$PLANTUML_SHA256" ] && ! verify_sha256 "$jar" "$PLANTUML_SHA256"; then echo "Cached PlantUML jar failed checksum verification; downloading a fresh copy." >&2; rm -f "$jar"; fi; if [ ! -s "$jar" ]; then mkdir -p "$cache"; tmp="$jar.tmp"; rm -f "$tmp"; if ! command -v curl >/dev/null 2>&1; then echo "PlantUML renderer not found and curl is unavailable for automatic download." >&2; exit 1; fi; echo "Downloading PlantUML $PLANTUML_VERSION renderer to $jar" >&2; curl -fL --retry 2 -o "$tmp" "https://github.com/plantuml/plantuml/releases/download/$PLANTUML_VERSION/plantuml.jar"; if [ -n "$PLANTUML_SHA256" ]; then verify_sha256 "$tmp" "$PLANTUML_SHA256" || { rm -f "$tmp"; exit 1; }; fi; mv "$tmp" "$jar"; fi; printf '%s\\n' "$jar"; }`;
}

function checksumSnippet() {
  return `calc_sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | awk '{print $1}'; elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'; else echo "Checksum verification requires sha256sum or shasum." >&2; exit 1; fi; }; verify_sha256() { actual="$(calc_sha256 "$1")"; expected="$2"; if [ "$actual" != "$expected" ]; then echo "Checksum mismatch for $1. Expected $expected, got $actual." >&2; return 1; fi; }`;
}

function ensurePdfLibsSnippet() {
  const checks = plantumlPdfDependencies
    .map(
      ({ fileName, sha256, url }) =>
        `if [ -s "$pdf_dir/${fileName}" ] && ! verify_sha256 "$pdf_dir/${fileName}" "${sha256}"; then echo "Cached PlantUML PDF dependency ${fileName} failed checksum verification; downloading a fresh copy." >&2; rm -f "$pdf_dir/${fileName}"; fi; if [ ! -s "$pdf_dir/${fileName}" ]; then tmp="$pdf_dir/${fileName}.tmp"; rm -f "$tmp"; echo "Downloading PlantUML PDF dependency ${fileName} to $pdf_dir/${fileName}" >&2; curl -fL --retry 2 -o "$tmp" "${url}"; verify_sha256 "$tmp" "${sha256}" || { rm -f "$tmp"; exit 1; }; mv "$tmp" "$pdf_dir/${fileName}"; fi`,
    )
    .join("; ");

  return `is_pdf_render() { for arg in "$@"; do if [ "$arg" = "-tpdf" ]; then return 0; fi; done; return 1; }; ensure_pdf_libs() { pdf_dir="$1"; if ! command -v curl >/dev/null 2>&1; then echo "PDF export with the cached PlantUML jar needs Batik/FOP sidecar libraries, but curl is unavailable for automatic download." >&2; exit 1; fi; ${checks}; }`;
}

function runPlantUmlSnippet() {
  return `run_plantuml() { if [ -n "\${PLANTUML_ZED_PLANTUML_BIN:-}" ]; then "$PLANTUML_ZED_PLANTUML_BIN" -failfast2 "$@"; elif command -v plantuml >/dev/null 2>&1; then plantuml -failfast2 "$@"; elif [ -n "\${PLANTUML_ZED_PLANTUML_JAR:-}" ] && [ -f "$PLANTUML_ZED_PLANTUML_JAR" ]; then "$java_bin" -jar "$PLANTUML_ZED_PLANTUML_JAR" -failfast2 "$@"; elif [ -n "\${PLANTUML_JAR:-}" ] && [ -f "$PLANTUML_JAR" ]; then "$java_bin" -jar "$PLANTUML_JAR" -failfast2 "$@"; else jar="$(ensure_plantuml_jar)"; if is_pdf_render "$@"; then ensure_pdf_libs "$(dirname "$jar")"; fi; "$java_bin" -jar "$jar" -failfast2 "$@"; fi; }`;
}

function findPumlSnippet() {
  return `find_puml() { find "$ZED_WORKTREE_ROOT" -type f \\( -name '*.puml' -o -name '*.plantuml' -o -name '*.pu' -o -name '*.wsd' -o -name '*.iuml' \\) -not -path '*/.git/*' -not -path '*/node_modules/*' -not -path '*/out/plantuml/*' "$@"; }`;
}

function workspaceRenderSnippet(format) {
  return `if [ -n "\${PLANTUML_ZED_PLANTUML_BIN:-}" ]; then find_puml -print0 | xargs -0 "$PLANTUML_ZED_PLANTUML_BIN" -failfast2 -t${format} -o "$out"; elif command -v plantuml >/dev/null 2>&1; then find_puml -print0 | xargs -0 plantuml -failfast2 -t${format} -o "$out"; elif [ -n "\${PLANTUML_ZED_PLANTUML_JAR:-}" ] && [ -f "$PLANTUML_ZED_PLANTUML_JAR" ]; then find_puml -print0 | xargs -0 "$java_bin" -jar "$PLANTUML_ZED_PLANTUML_JAR" -failfast2 -t${format} -o "$out"; elif [ -n "\${PLANTUML_JAR:-}" ] && [ -f "$PLANTUML_JAR" ]; then find_puml -print0 | xargs -0 "$java_bin" -jar "$PLANTUML_JAR" -failfast2 -t${format} -o "$out"; else jar="$(ensure_plantuml_jar)"; if [ "pdf" = "${format}" ]; then ensure_pdf_libs "$(dirname "$jar")"; fi; find_puml -print0 | xargs -0 "$java_bin" -jar "$jar" -failfast2 -t${format} -o "$out"; fi`;
}

function openCurrentPngSnippet() {
  return `base=$(basename "$ZED_FILE"); base="\${base%.*}"; if [ -f "$out/$base.png" ]; then if command -v open >/dev/null 2>&1; then open "$out/$base.png"; elif command -v xdg-open >/dev/null 2>&1; then xdg-open "$out/$base.png"; elif command -v powershell.exe >/dev/null 2>&1; then powershell.exe -NoProfile -Command "Start-Process '$out/$base.png'"; else echo "Exported $out/$base.png"; fi; else echo "Export finished. Multi-diagram output may use $out/\${base}_*.png"; fi`;
}
