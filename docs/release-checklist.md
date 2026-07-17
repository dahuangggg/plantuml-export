# Release Checklist

Use this before submitting to `zed-industries/extensions`.

## Source hygiene

- `grammars/`, `out/`, `target/`, `node_modules/`, and `vendor/` are ignored.
- No generated images or local course files are included in the extension source.
- `extension.toml` has the final repository URL, authors, version, and license.
- The extension does not duplicate an existing community extension without a
  clear reason; prefer an upstream PR when possible.

## Functional checks

- `npm test` passes.
- `npm run generate:tasks` leaves `languages/plantuml/tasks.json` unchanged.
- `npm run check:release-package` passes.
- `cargo check --target wasm32-wasip1` passes.
- GitHub Actions passes on macOS, Linux, and Windows.
- `zed: install dev extension` installs cleanly.
- Opening `.puml`, `.plantuml`, `.pu`, `.wsd`, and `.iuml` files selects the
  PlantUML language.
- Syntax highlighting loads without query errors in `~/Library/Logs/Zed/Zed.log`
  on macOS or the platform equivalent.
- `task: spawn` shows the PlantUML current-file and workspace export tasks.
- `PlantUML: export current file to PNG and open` opens the first generated PNG.
- `PlantUML: renderer health check` reports Java, PlantUML jar, and Graphviz
  status.
- Export tasks honor `PLANTUML_ZED_OUTPUT_DIR` and
  `PLANTUML_ZED_PLANTUML_JAR` in a smoke test.
- Managed cached-jar PDF export auto-downloads Batik/FOP sidecar jars and
  writes a non-empty PDF.
- A corrupted cached managed jar or PDF sidecar is rejected by checksum
  verification and re-downloaded.
- Markdown fenced code blocks labelled `plantuml` receive PlantUML highlighting
  through Zed's built-in fenced-code injection.
- Opening a broken `.puml` publishes a diagnostic through `plantuml-lsp`.
- PNG and SVG export succeed with `plantuml` on `PATH`, `PLANTUML_JAR`, and the
  automatic cached-jar fallback.
- PNG/SVG/PDF export is smoke-tested with a Unicode path.

## Documentation

- README documents local rendering, local LSP diagnostics, settings, and Zed API
  limits.
- Troubleshooting includes Java, Graphviz, missing renderer, checksum mismatch,
  PDF sidecar dependencies, and empty output failures.
- Known gaps are listed so users do not expect VS Code PlantUML parity.
