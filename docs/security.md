# Security and Network Model

## Trust inputs

- Project config is treated as repository-controlled. It may select a renderer
  mode/layout, add portable local include roots, and tighten `remoteIncludes` or
  enable `offline`, but it cannot provide local executable, JAR, Java, Graphviz,
  download, or trusted-origin paths. Non-managed selections can therefore invoke
  only tools the machine owner already authorized through user config or CLI.
- Zed `initialization_options` is also worktree-controlled. It accepts exactly
  portable `includePaths` and `remoteIncludes`; both are additive or
  restriction-only. It cannot set tools, `offline`, or `allowedRemoteUrls`, and
  unknown keys fail closed.
- User config and explicit CLI flags are machine-owner decisions and may select
  tools or absolute local include roots. Only user config may define
  `allowedRemoteUrls` and the starting remote-include policy. An allowed origin
  is a global machine authorization applied to every worktree.
- The internal ownership manifest is parsed strictly and may claim files only
  below the configured output directory.

All project/output/input paths are normalized and checked against canonical
ancestors. Discovery does not follow directory symlinks. A symlink or traversal
that leaves the worktree/output boundary fails before rendering or cleanup.

## Renderer process

Every render and syntax-check invocation clears inherited PlantUML security/include/URL variables plus
`JAVA_TOOL_OPTIONS`, `JDK_JAVA_OPTIONS`, and `_JAVA_OPTIONS` before applying the
resolved policy; otherwise these aggregate JVM option variables could inject
system properties after the helper resolved its policy. The worktree and
current source directory are automatic local read roots. Explicit `includePaths`
add canonical user roots or portable worktree-relative roots; they are not
required for ordinary includes inside the workspace.

The remote policy is:

- `remoteIncludes = "public"` (the default): use PlantUML `INTERNET` and permit
  its public HTTP(S) behavior, plus any user-trusted origins;
- `remoteIncludes = "allowlist"`: use `ALLOWLIST` and permit only
  user-configured `allowedRemoteUrls`;
- `remoteIncludes = "disabled"`, or any `offline = true`: use `ALLOWLIST` and
  remove all authorized remote origins.

Project config and Zed settings combine this policy by restriction, so a
repository cannot change a user's `disabled` policy back to `public` or turn
off the user's offline mode. `allowedRemoteUrls` entries are parsed as HTTP(S)
origins containing only a scheme, host, and optional port. A non-root path,
credentials, query string, fragment, or the PlantUML `;` separator is rejected;
the canonical value ends in `/`. Each entry authorizes every path on that origin
for every worktree. This broad effect is why the setting is a user-only, global
machine authorization: project config and Zed settings can restrict it but can
never add an origin. Export and saved-file syntax checks receive the same
effective policy.

The default `public` mode intentionally delegates destination checks to
PlantUML's upstream `INTERNET` profile so normal public includes work without
configuration. Its address and port restrictions reduce SSRF exposure, but they
are not a strong network sandbox, are not an absolute defense against DNS
rebinding, and may change with PlantUML. Run untrusted diagrams behind an
operating-system or network egress boundary when hard isolation is required.

Metadata is disabled by default. Rendering also passes PlantUML's
`--ignore-startuml-filename`; otherwise an `@startXYZ ../../name` directive can
escape `--output-dir` before the export transaction can inspect the result.

The tool executes only the selected renderer mode. There is no fallback that
could unexpectedly run a different binary. Managed mode downloads its pinned
Java runtime; explicit JAR mode uses the configured Java. Graphviz is never
bundled or downloaded. The configured Graphviz program is resolved to an
absolute path; the inherited `GRAPHVIZ_DOT` value is removed before that exact
path is applied. Renderer stdin is closed, captured output is bounded, and
cancellation or timeout terminates the associated process tree.

## Managed downloads

The native toolchain may fetch only the compiled-in PlantUML 1.2026.6 and
target-specific Temurin 21.0.11+10 URLs. It limits each compressed response to
64 MiB and verifies the compiled-in SHA-256 before extraction or rename. JRE
extraction occurs under a private staging directory with a 512 MiB expanded
limit and 4096-entry cap. Absolute/traversing paths, escaping links, hard links,
special files, and duplicates are rejected; safe internal license symlinks are
validated and materialized as regular files. The single top-level runtime is
marked and atomically installed only after its Java executable is verified.

These requests can occur only for an explicit export, check, or
`plantuml-export lsp` startup. LSP completes initialize/initialized first, then
performs the same locked installation and SVG/Smetana health check with managed
Java 21; failure exits as environment status on stderr and never becomes a
source diagnostic. Offline mode forbids missing managed-asset downloads and all
remote diagram includes; an installed cache that passes the reuse checks remains
usable. Graphviz is not required for this startup check.

The integrity guarantee has two distinct phases. During download and
installation, the exact JAR or JRE archive bytes are verified against the
compiled-in digest before installation or extraction. On later reuse, the JAR
is hashed again, while the extracted JRE is checked only for its exact
version/archive-digest marker, a real non-symlink runtime directory, and a real
executable Java launcher before the health process runs. The tool does not
re-hash every file in the extracted JRE on every invocation.

The managed cache is therefore trusted, user-scoped local state after a
successful installation. These checks detect a missing or structurally invalid
cache, but they are not a security boundary against another process already
running as the same operating-system user modifying JRE files. Re-hashing the
full runtime would add substantial startup I/O and would still leave a
check-to-execution race. Environments that treat same-user processes as hostile
must protect the cache and execution path with operating-system account,
permission, or sandbox isolation.

After an independent GitHub helper release, the Zed adapter may fetch only one
target-specific URL present in checked-in metadata. It verifies the exact
SHA-256 before installation. Concurrent Zed processes install safely without
sharing mutable staging or backup paths: every download uses a unique
staging directory, and an immutable checksum-addressed helper is installed with
create-if-absent semantics. If the host filesystem cannot create a hard link,
the adapter atomically renames to a unique final path instead. Every selected
final path is re-checked before launch, and an existing invalid candidate is
never deleted or overwritten. Unpublished, incomplete, malformed, or
placeholder metadata fails closed.

Zed's extension download API accepts a destination path but does not expose a
streaming byte-limit callback. The adapter therefore enforces its 64 MiB helper
limit immediately after Zed finishes writing the response and before checksum
acceptance or execution; this bounds accepted artifacts, but cannot stop an
oversized response from first consuming temporary download bandwidth and disk.
The embedded SHA-256 still prevents such a response from being installed or
executed.

## Zed export requests

The extension-managed helper advertises one LSP command,
`plantuml-export.export`. Its argument is strictly decoded as one local file URI
and one of `svg`, `png`, or `pdf`; unknown fields and commands are rejected. The
canonical input must be a regular file inside the active worktree, and its saved
content must equal the latest LSP document text. This prevents a forged command
from selecting an arbitrary path and prevents a dirty buffer from silently
exporting stale disk content.

One serial worker executes Code Action exports through the same in-process
runtime as the optional CLI, with at most one pending request. Further requests
receive an explicit busy response, and shutdown cancels the active renderer and
discards pending work. It does not start another `plantuml-export` executable or
use `PATH`. Success and failure are reported through standard LSP responses and
`window/showMessage`.

Zed forwards only `includePaths` and `remoteIncludes` from
`initialization_options`. Include paths must be portable and project-relative,
and the remote value can only restrict the resolved user policy. In particular,
Zed settings cannot supply `allowedRemoteUrls`, machine executables, or an
offline override. `remoteIncludes` never selects a remote rendering server; it
only controls include requests made by the local renderer.

## Local mutation

Exports are written to private render staging first. Only validated final SVG,
PNG, and PDF files enter the configured output tree; manifest, lock, staging,
journal, and backup files never appear there. Existing unmanaged targets are
never overwritten. One operating-system lock serializes writers per
workspace/output pair. Stale cleanup uses the prior internal manifest and only
for successfully re-rendered source-format pairs. Manifest schema v2 records
every output as `{ path, sha256 }`; replacement and stale cleanup require the
current bytes to match that digest, so external stale state cannot claim a user
replacement in a recreated output tree. Schema v1 is deliberately rejected
without migration while the project remains unreleased. A synced journal
records backup state, created directories, and expected installed-file hashes
before mutation. Under
`inputs[source].formats[format]`, each format owns its output paths and exact
tool/renderer/environment provenance; a partial export preserves untouched
sources and formats and cannot relabel them with the current invocation's
provenance. Re-exporting one format can remove only that format's stale
multi-page outputs, never another format owned by the same source. A synced
backups-complete marker is written before any installation begins, so a missing
backup after that phase is treated as ambiguous instead of as an
already-restored file. A command failure rolls back immediately; the next
writer recovers a process-interrupted transaction.
After an owned path is moved into the transaction backup, its type and digest
are checked again. Final files are installed with an atomic no-clobber hard
link, so a user or another tool racing a new target into place is never silently
overwritten.
Changed, missing, or unexpected files fail closed and leave the current
journal and backups for inspection instead of guessing recovery state.

The ownership manifest, writer lock, render staging, rollback journal, and
backups live below a private platform application-state root keyed by a stable
hash of the canonical workspace/output pair:

- macOS: `~/Library/Application Support/plantuml-export/exports/<workspace-hash>`
- Windows: `%LOCALAPPDATA%\plantuml-export\state\exports\<workspace-hash>`
- Linux: `${XDG_STATE_HOME:-~/.local/state}/plantuml-export/exports/<workspace-hash>`

The workspace state root, staging directory, and transaction backup root must
be real directories, never symlinks or Windows reparse points. State paths and
output paths are validated against their separate canonical boundaries, and
manifest ownership cannot claim an internal state file. Keeping these
boundaries separate ensures that user-visible output contains only final
artifacts while recovery data remains private to the tool. Output and state
must reside on the same filesystem; a cross-volume configuration is rejected
before rendering so transaction renames never silently degrade to copying.

## Privacy

There is no telemetry, server renderer, source upload, WebView, or self-update.
The native helper always renders the full diagram locally. GitHub may receive
ordinary helper/JRE/JAR download requests, and a remote include host receives
the resource request explicitly permitted by the effective policy; neither is
used as a diagram-rendering endpoint. PlantUML source and generated files remain
local.

## Reporting

Do not publish sensitive source, local paths, or private configuration in a
public issue. Reproduce security problems with a minimal synthetic worktree and
include the `health --json` output only after reviewing it for machine paths.
