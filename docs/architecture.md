# Architecture

## Product split

PlantUML Export has two release surfaces with one native behavior contract:

```text
Zed language assets and Wasm adapter
  └─ verified native helper as LSP
       ├─ Code Action -> execute export
       └─ edit/save diagnostics
                    │
                    ▼
     config -> discovery -> renderer -> staged export -> state commit

optional standalone CLI ────────────────┘
```

The root Rust crate compiles to the Zed Wasm extension. Native-only filesystem,
process, HTTP, and LSP dependencies live in `crates/plantuml-export`; they never
enter the Wasm build. Node is used only for repository tests and
release-metadata tooling.

## Zed integration

Language configuration, queries, and snippets live under `languages/plantuml/`
and `snippets/`. The adapter in `extension/src/lib.rs` is intentionally thin;
the root package points at that source so Zed can still compile the repository
root directly as a development extension.

For `plantuml-lsp`, it first reads `release/native-helper-release.json`.
Published metadata always selects the pinned extension-managed helper and never
consults `PATH`. Only unpublished source checkouts ask Zed for
`worktree.which("plantuml-export")` as a development bootstrap. Published
metadata must contain exactly one tag-specific GitHub URL and non-placeholder
SHA-256 for each of the six supported Rust targets. The adapter downloads one
uncompressed binary into a unique staging directory in its private working
directory and hashes it. Installation creates an immutable checksum-addressed
helper without replacing a shared path: an atomic hard link wins the canonical
name, while hosts without hard-link support atomically rename to a unique final
name. Concurrent installers therefore cannot share staging or backup files, and
every selected destination is re-hashed before launch. Unpublished metadata
without a local development helper, or incomplete metadata, fails closed.

Zed's documented download API writes into a
[private extension working directory](https://github.com/zed-industries/zed/blob/dde45ff09276331eb58419c3245d4a3ccb7534f6/crates/extension_api/wit/since_v0.8.0/extension.wit#L53-L178).
Language-server commands resolve paths in that private directory, so the
adapter can launch the downloaded helper without installing it on the user's
`PATH`.

The helper advertises `textDocument/codeAction` and the single
`plantuml-export.export` command. Zed keeps only commands listed by the server's
execute-command capability and forwards a selected action through
[`workspace/executeCommand`](https://github.com/zed-industries/zed/blob/dde45ff09276331eb58419c3245d4a3ccb7534f6/crates/project/src/lsp_store.rs#L5770-L5813).
SVG, PNG, and PDF actions therefore execute inside the same native helper/LSP process.
Export is intentionally handled by the helper process so the installed
extension never needs a second CLI from `PATH`. The standalone CLI remains an
optional terminal and CI surface.

The adapter also forwards Zed's `initialization_options` to the helper. That
worktree-controlled surface accepts exactly `includePaths` and
`remoteIncludes`. Include paths are additive, portable paths relative to the
worktree; `remoteIncludes` can only tighten the policy already selected by the
user config. Machine tool paths, `offline`, and `allowedRemoteUrls` are not
accepted through Zed settings, and unknown keys fail closed.

## Native command path

The native crate is organized by durable boundaries:

- `cli.rs`: public flags and typed enums;
- `config.rs`: root discovery, precedence, and the portable project schema;
- `discovery.rs`: ignore-aware, suffix-limited, deterministic input inventory;
- `renderer.rs`: explicit renderer command construction, process execution,
  health, output validation, and managed JRE/JAR integrity;
- `export_state.rs`: platform state location and per-workspace isolation;
- `export.rs`: staging, ownership planning, transactional replacement,
  rollback, and manifest serialization;
- `diagnostics.rs` and `lsp.rs`: structural/report diagnostics and cancellable
  stdio LSP;
- `runtime.rs`: production wiring shared by CLI commands.

The JSON envelope has `schemaVersion = 1`; the process contract is success `0`,
operation/source failure `1`, and usage/config/environment failure `2`.

## Configuration and trust

One `plantuml-export.toml` is loaded from the worktree root. Nested configs do
not cascade. Built-in defaults are refined by user config, project config,
explicit CLI overrides, and finally Zed initialization options. Scalar behavior
uses that precedence, while `includePaths` is an additive list and network
restrictions are monotonic: a project or Zed worktree can tighten a user policy
but cannot widen it.

Project config may describe portable behavior but cannot choose executable,
JAR, Graphviz, Java, or download paths. Project output/include paths must be
relative and cannot traverse with `..`. Machine-local paths belong to user
config or explicit CLI flags. Relative include paths are canonicalized inside
the root; an absolute user/CLI include is an explicit additional trust root.
Project config may select a stricter `remoteIncludes` value or enable `offline`,
but `allowedRemoteUrls` is a user-only, machine-global authorization applied to
every worktree. Zed `initialization_options` is narrower: it supports only
portable `includePaths` and a tightening `remoteIncludes`.

## Renderer strategy

Renderer selection is tagged, not a probing chain:

- `managed`: fixed PlantUML `1.2026.6` generic JAR plus target-specific Eclipse
  Temurin JRE `21.0.11+10`, all checksum-pinned;
- `binary`: exactly the configured executable;
- `jar`: exactly the configured JAR and Java executable.

Managed acquisition uses fixed HTTPS URLs, 64 MiB compressed-asset and 512 MiB
expanded-runtime limits, an entry-count cap, crash-released operating-system
cache locks, unique staging paths, SHA-256 verification, and same-cache atomic
installation. Archive paths are confined to one runtime root; safe internal
license links are materialized as regular files, while escaping links, hard
links, special files, duplicate entries, and traversal are rejected. Offline
mode never downloads. Temurin's notice, legal, and code-signature files remain
intact.

Acquisition integrity and cache-reuse integrity are deliberately separate. The
JAR is re-hashed before reuse. The extracted JRE is not fully re-hashed on every
startup: reuse requires the expected version/archive-digest marker, a real
non-symlink runtime directory, and a real executable Java launcher, followed by
the normal health check. This catches incomplete or structurally invalid state,
but not arbitrary post-install changes by another process running as the same OS
user. Full-tree verification would impose significant startup I/O and still
could not eliminate the time-of-check/time-of-use window before Java executes,
so same-user cache tampering belongs to the host permission/sandbox threat
boundary rather than the download verifier.

Every render and syntax-check process clears inherited security/include/URL allowlist values,
plus `JAVA_TOOL_OPTIONS`, `JDK_JAVA_OPTIONS`, and `_JAVA_OPTIONS`, then applies
an explicit policy so aggregate JVM options cannot override it. The worktree and
current source directory are always local read roots; resolved `includePaths`
add user- or worktree-authorized roots. With the default
`remoteIncludes = "public"`, an
online invocation uses PlantUML `INTERNET`; any user-configured
`allowedRemoteUrls` are added as explicit origins. `remoteIncludes = "allowlist"`
uses `ALLOWLIST` and only those user-configured origins for remote access.
`remoteIncludes = "disabled"` or `offline = true` uses `ALLOWLIST` and supplies
no authorized remote origin. User entries must contain only an HTTP(S) scheme,
host, and optional port; a non-root path, credentials, query, fragment, or `;`
is rejected, and the canonical origin ends in `/`. An entry authorizes every
path on that origin for every worktree. The same policy is used for export and
saved-file syntax checks.

The zero-configuration `public` mode deliberately relies on PlantUML's upstream
`INTERNET` address and port checks. This preserves ordinary public includes and
reduces SSRF exposure, but it is not a strong network sandbox and cannot promise
absolute isolation from DNS rebinding or a future upstream behavior change.

Metadata is disabled unless opted in. A configured Graphviz executable is
resolved to an absolute path, inherited `GRAPHVIZ_DOT` is cleared, and the
resolved path is set explicitly; Smetana removes the value without setting one.
Render commands force
`--ignore-startuml-filename` so source-level names cannot escape staging.
Smetana is the built-in default. Health therefore requires no Graphviz unless
`layout = "graphviz"` is explicit. Managed health always probes the pinned Java
21 runtime; explicit JAR health enforces Java 17 for SVG/PNG and Java 21 for PDF.

## Export transaction

Discovery canonicalizes the worktree and explicit files, applies ignore and
glob policy, skips symlink directories, and sorts normalized relative paths.
The export session then performs:

1. render each input into an isolated staging directory;
2. inventory every regular output and validate nonzero length and type magic;
3. map filenames into the mirrored output tree;
4. reject current-current, manifest-owner, and unmanaged-file conflicts;
5. merge prepared source-format entries into the manifest, preserving every
   untouched source and format's output ownership and provenance;
6. sync a journal containing old-file state, planned directories, and hashes of
   every file that may be installed;
7. rename existing owned files and manifest into a transaction backup, then
   sync a backups-complete marker;
8. hard-link staged files and the manifest into place only while each target is
   absent, unlink staging, then sync a commit marker;
9. restore backups if any commit step fails.

Manifest, writer lock, renderer staging, journal, markers, and backups live in
a private platform application-state directory keyed by the canonical
workspace/output pair. The user output tree therefore contains only final
artifacts. Cross-file replacement cannot be one filesystem syscall, so the
implementation uses same-filesystem renames plus an explicit rollback journal;
it rejects a state/output cross-volume pairing before rendering. The next
writer recovers an uncommitted schema-v2 journal before reading the manifest.
The backups-complete marker separates pre-install recovery from rollback; a
missing backup after that marker, hash mismatch, or unexpected nonempty
directory fails closed and preserves the journal and backup evidence for manual
inspection. The backup root must itself be a real non-reparse directory below
the private transaction staging directory. No filename is reserved inside the
user output tree.

Stale cleanup is limited to paths in the previous manifest for the successfully
re-rendered source and format. Manifest schema v2 uses
`inputs[source].formats[format]`; each format entry stores its own `outputs`,
`toolVersion`, `renderer`, and `environment`, with no global provenance fields.
Every `outputs` element is a `{ path, sha256 }` record. Existing managed files
must match that recorded digest before replacement or stale cleanup, so state
left outside a recreated output tree cannot claim user replacement bytes. This
digest is checked again after the old path has been atomically moved into the
transaction backup, and final installation never replaces an occupied path, so
a file raced into place at the commit boundary is preserved. This
unreleased development format intentionally rejects schema v1 instead of
migrating it.

A partial export clones untouched sources and formats and replaces only the
prepared source-format entries. Consequently, exporting PNG after SVG retains
both, while re-exporting SVG replaces only SVG provenance and removes only
stale SVG multi-page outputs. Unknown manifest shapes fail closed.
Default mode does not begin commit if any input failed. `--keep-going` commits
only independent successful inputs and retains structured failures.

## Diagnostics

LSP startup finishes initialize/initialized first, then resolves exactly the
configured renderer. This keeps the first pinned JRE/JAR download outside the
client's initialization deadline; requests sent meanwhile remain queued. It
checks SVG/Smetana and the pinned Java 21 runtime in managed mode, or the
configured Java in JAR mode. It never falls back or probes Graphviz.
Provisioning, offline, and health failures remain startup environment errors on
stderr.

In-memory structural diagnostics are scheduled with a 250 ms debounce and
publish only for the latest document generation. A save captures the real file
URI path and starts a 10-second PlantUML standard-report check. PlantUML 1.2026.6
returns from check-only mode before printing structured reports, so the check
uses a private temporary SVG/Smetana render with error images and metadata
disabled; its output directory is removed before the result is published. Each
document owns a cancellation token: edit, subsequent save, close, and LSP
shutdown cancel the old token; the process loop kills and waits for the child.
Stale or cancelled completions never publish.

Saved documents expose SVG, PNG, and PDF Code Actions. The LSP validates that
the URI resolves to a regular file inside the worktree and that its in-memory
text matches disk before enqueueing one serial export worker. Dirty buffers fail
with a visible save-first message instead of exporting stale content. The worker
calls the same native runtime as the CLI and reports completion through
`window/showMessage`; no `plantuml-export` subprocess is spawned.

Process stdin is closed. Stdout and stderr are drained concurrently and retain
at most 1 MiB per stream. Timeout/cancellation terminates the renderer process
tree (Unix process group or Windows Job Object) before joining pipe readers.
Only parsed PlantUML source reports become source diagnostics. Spawn, Java,
renderer, checksum, and timeout problems remain environment/status output.

## Release and checksum sequence

The tag workflow builds on six host-native GitHub runners and publishes raw
binaries plus `SHA256SUMS`; GitHub artifact attestations bind them to the tagged
workflow. Actions are pinned by commit and the publish job has the only write
permissions.

For RC1, the two GNU/Linux binaries are built and smoke-tested on Ubuntu 24.04;
that runner/glibc environment is the verified Linux compatibility boundary.

Checksums cannot be embedded in the same source commit before that commit's six
binaries exist. The independent toolchain therefore uses a two-step sequence:

1. publish `v0.1.0-rc.1` with metadata still `unpublished`;
2. download and verify its `SHA256SUMS`, generate the six-entry metadata table,
   and commit that table for the Zed adapter.

The generator rejects missing, extra, duplicate, uppercase/placeholder, or
wrongly named entries. No reproducible-build assumption is used to guess
cross-platform linker output.

## Network and privacy boundary

There is no server-renderer mode, telemetry, source upload, or self-update. A
full diagram is always rendered by the local native helper; `remoteIncludes`
only controls resources fetched by that local PlantUML process. Network access
is limited to the checksum-pinned helper/JRE/JAR acquisition and remote include
requests allowed by the effective `public` or `allowlist` policy. Diagram source
and generated artifacts are not submitted to a rendering service and remain on
the local machine. Deployments that require hard egress isolation for untrusted
diagrams must enforce it outside this process at the operating-system or network
boundary.
