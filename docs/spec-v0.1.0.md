# PlantUML Export v0.1.0 Specification

## Objective

Ship `PlantUML Export` as an independent, GitHub-first Zed extension and native
toolchain. The v0.1.0 line prioritizes deterministic local export, actionable
diagnostics, and supply-chain integrity over live preview or broad IDE feature
parity.

The extension ID is `plantuml-export`, the display name is `PlantUML Export`,
and the native executable is `plantuml-export`. The first public candidate is
`v0.1.0-rc.1`; creating or pushing that release requires explicit approval
after the exact commit and verification evidence are presented.

## Product Boundary

### Included

- Self-contained PlantUML language configuration, grammar declaration, queries,
  snippets, runnables, tasks, and a diagnostic-only language server.
- A native Rust CLI with `export`, `check`, `health`, `version`, and internal
  `lsp` commands.
- Local PNG, SVG, and PDF export through one of three explicit renderer modes:
  `managed` (default), `binary`, or `jar`.
- Current-file and workspace exports from saved standalone PlantUML files.
- GitHub Release artifacts for macOS, Linux, and Windows on both x86_64 and
  aarch64.

### Excluded

- Zed Store submission, coordination with the existing PlantUML extension, or
  any other public issue/PR during v0.1.0 implementation.
- Embedded/live preview, WebViews, server rendering, telemetry, source upload,
  export-on-save, file watching, and unsaved-buffer export.
- Markdown fenced-block export or Markdown rewriting. Fenced PlantUML remains a
  syntax-highlighting feature only.
- Package-manager distribution, self-update, bundled Java, bundled Graphviz,
  Apple notarization, and Windows Authenticode.
- Completion, hover, definition, rename, formatting, and code actions.

## Public Commands and Contracts

```text
plantuml-export export [INPUTS...] [--workspace] [--root PATH]
                       [--config PATH] [--format svg|png|pdf]
                       [--out-dir PATH] [--layout graphviz|smetana]
                       [--keep-going] [--require-input] [--json]
plantuml-export check [INPUTS...] [--workspace] [--root PATH]
                      [--config PATH] [--json]
plantuml-export health [--root PATH] [--config PATH] [--json]
plantuml-export version [--json]
plantuml-export lsp
```

- Human-readable output is the default; `--json` writes one stable JSON result
  document to stdout and keeps diagnostics/progress off stdout.
- Exit code `0` means success, `1` means an expected operation or source
  failure, and `2` means usage/configuration/environment failure.
- `export` defaults to SVG and `out/plantuml`.
- Legacy unreleased v0.0.1 options produce explicit migration errors instead of
  being silently reinterpreted. Stable flag names such as `--format`,
  `--out-dir`, and `--workspace` are retained where their semantics still fit.

## Configuration

The canonical project configuration is `plantuml-export.toml` at the worktree
root. There is exactly one project configuration per worktree; nested and
cascading project configurations are not supported.

Configuration precedence is:

1. explicit CLI or Zed invocation values;
2. project `plantuml-export.toml`;
3. user configuration;
4. built-in defaults.

`--root` selects a worktree explicitly. Without it, the CLI uses the Git root
containing the current directory, or the current directory when no Git root is
available. `--config` selects a project configuration explicitly.

Project configuration is portable and may contain format, output directory,
layout, include paths, include/exclude globs, security policy, and metadata
policy. Executable paths, JAR paths, Java paths, and download URLs are user-only
configuration. Zed project settings are read only after worktree trust.

## Renderer Contract

### Modes

- `managed` is the default and never silently falls back to another mode.
- `binary` requires a user-configured PlantUML executable.
- `jar` requires a user-configured PlantUML JAR and Java executable.

Managed mode pins PlantUML `1.2026.6` and the official generic JAR SHA-256:

```text
89948f14c93756c7a3fb7b69078ff37e8489fd79dd430c582b931e2f65358690
```

The version, URL, and checksum are tied to the tool release and cannot be
overridden by a project. Managed assets are fetched only after an explicit
export or setup action. Downloads use a fixed HTTPS URL, a per-asset lock,
checksum verification, a temporary file, and atomic replacement. Offline mode
disables downloads and returns an actionable environment error.

Managed PNG/SVG require Java 17 or newer. Managed PDF requires Java 21 or newer.
The tool does not download Java. Graphviz remains user-installed and is the
default layout engine. Smetana is explicit (`layout = "smetana"`) and never a
fallback. Export verification checks that every expected output exists, is
non-empty, and matches the requested file type; process exit code alone is not
sufficient.

## Security and Privacy

- PlantUML runs with `ALLOWLIST` security by default.
- Local reads are restricted to the source directory, worktree root, and
  explicit include paths. Remote includes are disabled by default.
- Project configuration cannot select executable paths or download URLs.
- Source and generated output never leave the machine. The tool has zero
  telemetry and no server-renderer mode.
- The managed network surface is auditable: GitHub Release download of the
  native helper by the Zed extension, and the fixed PlantUML JAR download after
  explicit managed-renderer use.
- Rendering uses PlantUML's metadata suppression by default. Source metadata is
  embedded only when `embed_source_metadata = true`.

## Export Session

Supported source suffixes are `.puml`, `.plantuml`, `.pu`, `.iuml`, and `.wsd`.
Only saved disk content is exported.

- Workspace discovery respects `.gitignore`, `.ignore`, and global Git ignore,
  then applies explicit include/exclude globs.
- Discovery never enters `.git`, the configured output directory, or the tool
  cache; it never follows directory symlinks; results are deterministically
  sorted.
- An interactive invocation with no inputs is a no-op. `--require-input` turns
  the same condition into an error for automation.
- Output paths mirror the workspace-relative source path. For example,
  `docs/auth/login.puml` becomes
  `out/plantuml/docs/auth/login.svg`.
- A source may generate multiple files (multiple blocks or `newpage`). The
  manifest records the exact source-to-output mapping.
- Conflicting output ownership is an error.
- Rendering occurs in staging. Every output is validated before atomic
  replacement. Cleanup removes only outputs previously owned by that source;
  unrelated files are never deleted.
- Workspace export is all-or-nothing by default. `--keep-going` permits partial
  success but still exits nonzero and reports every failure in JSON.
- The manifest records the tool version, renderer version, Java version,
  Graphviz version when used, OS/architecture, input identity, and outputs.
  Cross-platform behavior and paths are deterministic; image bytes are not
  promised identical across fonts or Graphviz installations.

## Diagnostics and LSP

The native helper provides a diagnostic-only stdio LSP.

- On edit, an in-memory structural pass runs after approximately 250 ms of
  debounce.
- On save and explicit `check`, the helper runs PlantUML syntax validation with
  the real source path and include context, a 10-second timeout, cancellation,
  and latest-result-wins stale suppression.
- PlantUML standard report output (`-stdrpt:1` or its stable equivalent) is
  parsed into exact one-based source lines and converted to LSP ranges.
- Missing Java, renderer, or Graphviz is an environment/health problem, not a
  fabricated source diagnostic.

## Zed Integration

The Wasm extension is a thin adapter. It resolves the six supported
platform/architecture pairs, downloads the pinned native helper from the
matching GitHub Release asset, verifies its embedded SHA-256, installs it
atomically, and launches `plantuml-export lsp`. It contains no Node runtime
dependency and no embedded JavaScript server.

Language tasks and runnables invoke the same public CLI contract. Any limitation
in Zed's task environment must be documented explicitly and handled through a
stable, cross-platform launcher contract rather than a generated giant shell
script.

## Release Contract

The GitHub tag workflow builds these six assets:

- `plantuml-export-aarch64-apple-darwin`
- `plantuml-export-x86_64-apple-darwin`
- `plantuml-export-aarch64-unknown-linux-gnu`
- `plantuml-export-x86_64-unknown-linux-gnu`
- `plantuml-export-aarch64-pc-windows-msvc.exe`
- `plantuml-export-x86_64-pc-windows-msvc.exe`

It publishes a checksum manifest and GitHub Artifact Attestations/provenance.
The extension pins exact helper URLs and SHA-256 values. A clean checkout must
contain every test fixture and document required by CI; generated grammars,
local build output, cached renderers, and conversation backups remain excluded.

## Testing Strategy

- Unit tests cover configuration precedence, path safety, diagnostics, manifest
  ownership, renderer selection, output validation, JSON schemas, and exit
  codes.
- Integration tests use real temporary worktrees and fake renderer processes to
  verify transaction, timeout, cancellation, multi-output, and failure paths
  without network access.
- Managed-renderer smoke tests verify the pinned JAR checksum and real output
  types when Java/Graphviz prerequisites are available.
- CI runs clean-checkout tests and native helper smoke tests on macOS, Linux,
  and Windows, plus cross-build/release checks for all six targets.
- Manual dev-extension QA covers language detection, queries, snippets,
  diagnostics, tasks, helper bootstrap, offline behavior, and Unicode paths.

## Success Criteria

v0.1.0 is ready for release review only when:

1. a clean checkout passes every hermetic test;
2. the native CLI and LSP satisfy the documented command/config/diagnostic
   contracts without Node;
3. export sessions are transactional and ownership-safe across documented
   success and failure paths;
4. the Zed adapter launches the verified native helper on every supported
   platform contract;
5. release workflows produce the six named assets, checksums, and provenance;
6. documentation and migration guidance match implemented behavior;
7. the final audit maps every requirement above to code and test evidence.

## Open Questions

No product-scope questions remain for v0.1.0. Implementation discoveries that
conflict with a documented Zed or platform capability must be surfaced with
source evidence before changing this specification.
