# Architecture

## Goals

This extension provides practical PlantUML support in Zed without depending on
VS Code-style WebViews or command APIs. The first community-ready version should
make standalone `.puml` editing and export reliable before attempting live
preview.

## Zed integration

Language support lives under `languages/plantuml/`:

- `config.toml` maps PlantUML file suffixes to the language.
- `highlights.scm`, `brackets.scm`, `indents.scm`, `folds.scm`, and
  `outline.scm` provide editor behavior.
- `runnables.scm` marks `@startuml` as a runnable export entry.
- `tasks.json` provides export tasks from the language extension itself.

This avoids asking users to copy `.zed/tasks.json` into every workspace.
`tasks.json` is generated from `src/task-commands.mjs` by
`scripts/generate-plantuml-tasks.mjs`, so renderer-task maintenance happens in
tested JavaScript rather than by hand-editing duplicated JSON shell strings.
The generated tasks remain self-contained for community extension installs and
can be configured through environment variables such as
`PLANTUML_ZED_OUTPUT_DIR`, `PLANTUML_ZED_PLANTUML_VERSION`,
`PLANTUML_ZED_PLANTUML_BIN`, `PLANTUML_ZED_PLANTUML_JAR`, and
`PLANTUML_ZED_JAVA`; health checks also honor `PLANTUML_ZED_GRAPHVIZ_DOT`.

The Rust/Wasm extension shell lives in `src/lib.rs` and registers
`plantuml-lsp`. It starts Zed's bundled Node runtime with the embedded
`src/lsp-stdio.cjs` script, then passes `plantuml-lsp` workspace settings as LSP
initialization options. This gives the extension a native Zed integration point
without depending on a VS Code-style command API.

No explicit extension capability entry is required for this shell today. The
Rust code does not call `zed_extension_api::process::Command`,
`zed_extension_api::download_file`, or `zed_extension_api::npm_install_package`;
the user-facing exports run as Zed tasks, while the language server command uses
Zed's normal language-server hook and bundled Node path.

Markdown fenced-code highlighting relies on Zed's built-in Markdown injection:
when a fence is labelled `plantuml`, Zed can resolve it to the PlantUML language
registered by this extension. The extension intentionally does not define its
own Markdown language because that could interfere with Zed's normal Markdown
support.

## Renderer strategy

Zed tasks use local rendering because task templates cannot directly call a
helper inside the extension install directory. They resolve renderers in this
order:

1. `plantuml` on `PATH`
2. `PLANTUML_JAR`
3. automatically cached PlantUML `v1.2025.4` jar

The cached jar is stored outside the workspace:

- macOS: `~/Library/Caches/zed-plantuml/plantuml-v1.2025.4.jar`
- other platforms:
  `${XDG_CACHE_HOME:-$HOME/.cache}/zed-plantuml/plantuml-v1.2025.4.jar`

This keeps export setup invisible for typical users while avoiding a large jar
inside the extension package or the user's project. The first export or renderer
health check needs network access to GitHub if no local renderer is available.
When the managed jar is used for PDF export, the task/CLI also downloads the
Batik/FOP sidecar jars required by PlantUML's `-tpdf` path into the same cache
directory. User-specified jars are left untouched because their owning directory
may not be writable or safe for the extension to modify.
The pinned PlantUML jar and the managed PDF sidecar downloads include SHA-256
metadata. The CLI verifies downloads in Node before writing the final cache
file, and generated Zed tasks verify cached and freshly downloaded assets with
`sha256sum` or `shasum`.

The Node CLI in `scripts/plantuml-export.mjs` is a development and power-user
tool. New shared modules in `src/config.mjs`, `src/renderer.mjs`, and
`src/diagnostics.mjs` define the local-only settings model, renderer decisions,
managed-asset downloads, and diagnostic parsing used by the mature extension
path.

Server rendering is intentionally out of scope for this phase. Keeping rendering
local avoids sending private diagram source to a remote service and keeps the
settings model smaller.

## Diagnostics

`src/lsp-stdio.cjs` implements a small stdio JSON-RPC language server. It
responds to `initialize`, tracks open documents, and publishes diagnostics for
structural issues such as missing `@end...` directives or non-empty files
without an `@start...` directive. On save, it attempts a local PlantUML
`-failfast -checksyntax` run when a binary or jar is available, then maps
`Error line N in file:` output to LSP diagnostics. `src/diagnostics.mjs` keeps
the parser-output mapping testable outside the stdio server.

## Current limitations

- Embedded live preview is blocked by Zed's lack of a stable WebView/custom
  preview extension API.
- Markdown fenced-code aliases such as `puml` and `uml` are not mapped yet;
  `plantuml` is the supported fence label.
- Current Tree-sitter support is intentionally conservative because the
  available PlantUML grammar only models a subset of PlantUML syntax.
- Diagnostics are local-only and depend on a configured or discoverable
  PlantUML renderer for save-time syntax checks.
- PDF export through user-managed jars still depends on the user providing
  PlantUML's PDF sidecar libraries next to that jar.
- Custom `PLANTUML_ZED_PLANTUML_VERSION` task downloads are only checksum
  verified when `PLANTUML_ZED_PLANTUML_SHA256` is also set.
- Export commands remain task-based until Zed exposes a public arbitrary
  editor-command API for extensions.
