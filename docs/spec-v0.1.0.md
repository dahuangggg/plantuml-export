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
  snippets, and SVG/PNG/PDF export Code Actions backed by the native language
  server.
- A native Rust CLI with `export`, `check`, `health`, `version`, and internal
  `lsp` commands.
- Local PNG, SVG, and PDF export through one of three explicit renderer modes:
  `managed` (default), `binary`, or `jar`.
- A checksum-pinned Eclipse Temurin JRE and PlantUML JAR managed automatically
  on all six supported native targets, with Smetana as the no-Graphviz default.
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
- Package-manager distribution, self-update, bundled Graphviz, custom `jlink`
  runtimes, Apple notarization, and Windows Authenticode.
- Completion, hover, definition, rename, and formatting.

## Public Commands and Contracts

```text
plantuml-export export [INPUTS...] [--workspace] [--root PATH]
                       [--config PATH] [--format svg|png|pdf]
                       [--out-dir PATH] [--layout graphviz|smetana]
                       [--include-path PATH]
                       [--keep-going] [--require-input] [--json]
plantuml-export check [INPUTS...] [--workspace] [--root PATH]
                      [--config PATH] [--include-path PATH] [--json]
plantuml-export health [--root PATH] [--config PATH] [--json]
plantuml-export version [--json]
plantuml-export lsp
```

- Human-readable output is the default; `--json` writes one stable JSON result
  document to stdout and keeps diagnostics/progress off stdout.
- Exit code `0` means success, `1` means an expected operation or source
  failure, and `2` means usage/configuration/environment failure.
- `export` defaults to SVG and `out`.
- `--include-path` is additive and is available to both `export` and `check`, so
  syntax validation and rendering resolve the same local includes.

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

Project configuration is portable and may contain renderer mode, format, output
directory, layout, include paths, include/exclude globs, the `remoteIncludes`
policy, and metadata policy. It may select a mode but cannot supply executable,
JAR, Java, Graphviz, or download paths; those paths and trusted
`allowedRemoteUrls` are user-only configuration.

`includePaths` is additive across user configuration, project configuration,
CLI `--include-path`, and Zed initialization options; later scopes do not erase
earlier include roots. Project and Zed include paths must be portable,
worktree-relative paths without `..`. Absolute include roots are machine-owner
decisions available only through user configuration or the standalone CLI.

`remoteIncludes` is a string enum with `public`, `allowlist`, and `disabled`.
The built-in default is `public`. User, project, and Zed scopes combine
monotonically as `public < allowlist < disabled`: repository-controlled project
configuration and Zed worktree settings may tighten network access but can
never loosen a stricter user choice. `offline = true` is also monotonic and
forces remote includes disabled in addition to preventing managed downloads.

`allowedRemoteUrls` is valid only in the machine-owner user configuration. Each
entry is an HTTP(S) origin containing only scheme, host, and optional port. A
non-root path, credentials, query, fragment, or `;` is invalid; the normalized
value ends in `/`. An entry authorizes every path at that origin and is a global
machine authorization applied to every worktree. Neither project configuration
nor Zed initialization options can add an origin.

Zed forwards the object under
`lsp.plantuml-lsp.initialization_options` directly to the native helper during
LSP initialization. The strict Zed schema accepts only `includePaths` and
`remoteIncludes`; it rejects machine-local tools, `offline`, and
`allowedRemoteUrls`. Initialization options are applied after configuration is
resolved and before renderer preparation.

```json
{
  "lsp": {
    "plantuml-lsp": {
      "initialization_options": {
        "remoteIncludes": "disabled",
        "includePaths": ["docs/includes"]
      }
    }
  }
}
```

There is deliberately no compatibility migration before the first release.
The old `security` setting has been removed, and a boolean `remoteIncludes` is
invalid; callers must use the three string values above.

## Renderer Contract

### Modes

- `managed` is the default and never silently falls back to another mode.
- `binary` requires a user-configured PlantUML executable.
- `jar` requires a user-configured PlantUML JAR and Java executable.

Managed mode pins PlantUML `1.2026.6`, Eclipse Temurin JRE `21.0.11+10`, the
six target-specific JRE URLs/checksums, and the official generic JAR SHA-256:

```text
89948f14c93756c7a3fb7b69078ff37e8489fd79dd430c582b931e2f65358690
```

The versions, URLs, and checksums are tied to the tool release and cannot be
overridden by a project. Managed assets are fetched only after an explicit
export, check, or `plantuml-export lsp` startup. Downloads use fixed HTTPS
URLs, per-asset operating-system locks that release after a crash, strict size
limits, checksum verification, safe staging extraction, and atomic replacement.
The full Temurin notice/legal tree is preserved. Offline mode disables downloads
and remote includes, and returns an actionable environment error when an
uncached managed asset is required.

The checksum guarantee applies to the downloaded JAR and JRE archive before
installation. On cache reuse, the JAR is hashed again; the extracted JRE is
validated by its exact version/archive-digest marker, non-symlink runtime root,
and executable Java launcher, but its full extracted tree is not re-hashed on
every invocation. The cache is trusted user-scoped local state after successful
installation. This detects missing or structurally invalid state, not arbitrary
post-install modification by another process with the same OS-user authority.
Full-tree hashing would add startup I/O without removing the final
check-to-execution race, so deployments requiring protection from same-user
processes must enforce it with host permissions or sandboxing.

Managed PNG/SVG/PDF require no user-installed Java. Managed mode selects only
the Java 21 runtime installed from its pinned archive and validated by the cache
reuse checks described above. Smetana is the default and requires no Graphviz.
Graphviz remains user-installed only for explicit `layout = "graphviz"` and is
never a fallback. Explicit `jar` mode requires Java 17 for PNG/SVG and Java 21
for PDF. Export verification checks that every expected output exists, is
non-empty, and matches the requested file type; process exit code alone is not
sufficient.

## Security and Privacy

- Every renderer process clears inherited PlantUML security, include, and URL
  allowlist values plus `JAVA_TOOL_OPTIONS`, `JDK_JAVA_OPTIONS`, and
  `_JAVA_OPTIONS` before applying the resolved policy, so aggregate JVM options
  cannot override it.
- `remoteIncludes = "public"` is the default and maps to PlantUML `INTERNET`.
  Ordinary public HTTP/HTTPS includes work without configuration, while
  loopback, private-network, link-local, raw-IP, and non-web-port destinations
  are checked by PlantUML upstream. These checks reduce SSRF exposure but are
  not a strong network sandbox or an absolute defense against DNS rebinding.
- `allowlist` maps to PlantUML `ALLOWLIST` and permits only origins supplied by
  the machine-owner user configuration. Each origin grants all of its paths in
  every worktree. `disabled`, and every offline invocation, applies `ALLOWLIST`
  without an authorized remote origin.
- Local reads remain restricted to the source directory, worktree root, and
  explicit include paths in every remote mode.
- Project configuration and Zed worktree settings cannot select executable
  paths, download URLs, or trusted remote origins, and cannot loosen the
  user's resolved remote policy.
- The tool has zero telemetry, no server-renderer mode, and no source-document
  upload. A remote include host receives an ordinary HTTP request and may
  observe its URL, client address, and standard HTTP metadata; rendering and
  generated output remain local.
- The network surface is auditable: diagram-requested public or user-allowlisted
  includes, the GitHub Release download of the native helper by Zed, and fixed
  Temurin JRE plus PlantUML JAR downloads after explicit managed export, check,
  or LSP startup.
- Rendering uses PlantUML's metadata suppression by default. Source metadata is
  embedded only when `embedSourceMetadata = true`.

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
  `out/docs/auth/login.svg`.
- The configured output tree contains final SVG, PNG, and PDF artifacts only.
  The ownership manifest, writer lock, render staging, rollback journal, and
  backups are internal state and never appear beneath the output directory.
- A source may generate multiple files (multiple blocks or `newpage`). The
  manifest schema v2 records the exact source-to-output mapping independently
  for every format under `inputs[source].formats[format]`. Each output is a
  `{ path, sha256 }` record, and each format entry also records its own tool
  version, renderer mode/version, and environment; there is no global
  provenance that can misdescribe retained entries.
- Conflicting output ownership is an error.
- Rendering occurs in staging. Every output is validated before atomic
  replacement. A managed file must still match its manifest SHA-256 before it
  can be replaced or removed. Cleanup removes only outputs previously owned by
  that source and format; unrelated files and the same source's other formats
  are never deleted. Re-exporting one format removes only its stale multi-page
  outputs.
- Writers to the same output directory are serialized. Before final-path
  mutation, a synced rollback journal records backups, created directories, and
  expected SHA-256 values; the next invocation recovers an interrupted commit
  and refuses to delete a changed file. A moved backup is re-hashed before any
  install, and staged files use atomic no-clobber hard links, so a target
  created or replaced after planning is preserved.
- Internal export state is keyed by a stable hash of the workspace/output pair
  and stored under the platform application-state root:
  - macOS:
    `~/Library/Application Support/plantuml-export/exports/<workspace-hash>`;
  - Windows:
    `%LOCALAPPDATA%\plantuml-export\state\exports\<workspace-hash>`;
  - Linux:
    `${XDG_STATE_HOME:-~/.local/state}/plantuml-export/exports/<workspace-hash>`.
- The output and application-state directories must be on the same filesystem;
  a cross-volume configuration fails before rendering or output mutation.
- Workspace export is all-or-nothing by default. `--keep-going` permits partial
  success but still exits nonzero and reports every failure in JSON.
- Each manifest format entry records the tool version, renderer version, Java
  version, Graphviz version when used, OS/architecture, and outputs from the
  invocation that last prepared that source-format pair. Partial and
  single-input exports retain all fields of untouched sources and formats and
  replace provenance only for successfully prepared source-format pairs.
  Exporting SVG, PNG, and PDF for one source therefore retains all three.
- The manifest schema is `inputs[source].formats[format]`; each source-format
  pair is an independent ownership record whose `outputs` entries contain the
  normalized path and lowercase SHA-256. Schema v1 is rejected without
  migration because v0.1.0 has not been released. Cross-platform behavior and
  paths are deterministic; image bytes are not promised identical across fonts
  or Graphviz installations.

## Diagnostics and LSP

The native helper provides export Code Actions and diagnostics over stdio.

- `textDocument/codeAction` returns SVG, PNG, and PDF actions for local saved
  PlantUML files. Zed forwards the selected action as the advertised
  `plantuml-export.export` command through `workspace/executeCommand`.
- Execute-command arguments contain exactly one file URI and output format. The
  helper rejects non-file URIs, unknown fields or formats, paths outside the
  worktree, and buffers whose in-memory text differs from saved disk content.
- Exports run on one serial worker and call the native runtime directly. They do
  not spawn another `plantuml-export` process or search `PATH`.
- LSP startup completes the protocol handshake before preparing the explicitly
  selected renderer, so a 66–78 MiB first-use managed download is not mistaken
  for an initialization timeout. It then applies the same download/checksum/
  offline policy as `check` and checks SVG/Smetana health plus the selected
  Java (pinned Java 21 in managed mode). Graphviz is not required. Failure is
  an environment exit on stderr after the handshake.
- On edit, an in-memory structural pass runs after approximately 250 ms of
  debounce.
- On save and explicit `check`, the helper runs PlantUML syntax validation with
  the real source path and include context, a 10-second timeout, cancellation,
  and latest-result-wins stale suppression.
- Explicit `check --include-path` and Zed `includePaths` feed the same additive
  include-path resolution used by export, so diagnostics cannot silently use a
  narrower include context than rendering.
- PlantUML standard report output (`-stdrpt:1`) is collected through an
  isolated, metadata-free SVG/Smetana diagnostic render and parsed into exact
  one-based source lines and LSP ranges. PlantUML 1.2026.6 check-only mode does
  not emit the structured report, so no generated diagnostic asset is exposed
  ([command path](https://github.com/plantuml/plantuml/blob/v1.2026.6/src/main/java/net/sourceforge/plantuml/Run.java#L342-L373),
  [report format](https://github.com/plantuml/plantuml/blob/v1.2026.6/src/main/java/net/sourceforge/plantuml/StdrptV1.java#L65-L75)).
- Missing Java or renderer state is an environment/health problem, not a
  fabricated source diagnostic. Graphviz is outside the syntax/LSP path.

## Zed Integration

The Wasm extension is a thin adapter. A PATH helper is supported only as an
unpublished source-checkout bootstrap. After published checksum metadata
exists, the adapter ignores PATH, resolves the six supported
platform/architecture pairs, downloads the pinned native helper from the
matching GitHub Release asset, verifies its embedded SHA-256, installs it
atomically in Zed's private extension directory, and launches
`plantuml-export lsp`. It contains no Node runtime dependency and no embedded
JavaScript server.

The same native helper/LSP process handles export Code Actions, so a released
extension never requires a separately installed CLI. The standalone CLI remains
an optional terminal/CI surface. Static PATH tasks and runnables are excluded
because they would reintroduce a second-install prerequisite.

The adapter forwards
`lsp.plantuml-lsp.initialization_options` without a custom wrapper. The helper
strictly decodes only portable `includePaths` and the restrictive
`remoteIncludes` mode before completing initialization; the command-line
`--root` remains the sole worktree authority. Changing initialization options
requires restarting the PlantUML language server.

## Release Contract

The GitHub tag workflow builds these six assets:

- `plantuml-export-aarch64-apple-darwin`
- `plantuml-export-x86_64-apple-darwin`
- `plantuml-export-aarch64-unknown-linux-gnu`
- `plantuml-export-x86_64-unknown-linux-gnu`
- `plantuml-export-aarch64-pc-windows-msvc.exe`
- `plantuml-export-x86_64-pc-windows-msvc.exe`

The RC1 Linux compatibility contract is the Ubuntu 24.04 GNU/glibc environment
used by the host-native build and managed-renderer smoke jobs. Other GNU/Linux
distributions may work but are not claimed as verified RC1 targets.

It publishes a checksum manifest and GitHub Artifact Attestations/provenance.
Because those hashes do not exist before the six binaries are built, a
follow-up commit generates the extension's exact helper URLs and SHA-256 table
from RC1 `SHA256SUMS`; placeholders are rejected. A clean checkout must
contain every test fixture and document required by CI; generated grammars,
local build output, cached renderers, and conversation backups remain excluded.

## Testing Strategy

- Unit tests cover configuration precedence, additive include roots, the
  public/allowlist/disabled remote policy, offline override, Zed initialization
  option restrictions, path safety, diagnostics, manifest ownership, renderer
  selection, output validation, JSON schemas, and exit codes.
- Integration tests use real temporary worktrees and fake renderer processes to
  verify transaction, timeout, cancellation, multi-output, and failure paths
  without network access.
- A real stdio integration test removes `plantuml-export` from `PATH`, requests
  an export Code Action, executes it through the helper, and verifies the output
  plus its internal state manifest while confirming that the output tree
  contains no bookkeeping files.
- Managed-renderer smoke tests clear external Java/Graphviz assumptions,
  provision the pinned JRE/JAR, verify their checksums, and validate real SVG,
  PNG, and PDF output types through Smetana.
- CI runs clean-checkout tests on macOS, Linux, and Windows plus host-native
  build/test checks for all six targets. The tag workflow runs a
  real managed SVG/PNG/PDF smoke test on each of the six host-native release
  binaries before upload.
- Manual dev-extension QA covers language detection, queries, snippets,
  diagnostics, export actions, helper bootstrap, offline behavior, and Unicode
  paths.

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
6. documentation matches implemented behavior and the clean first-release
   schema;
7. the final audit maps every requirement above to code and test evidence.

## Open Questions

No product-scope questions remain for v0.1.0. Implementation discoveries that
conflict with a documented Zed or platform capability must be surfaced with
source evidence before changing this specification.
