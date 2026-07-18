# PlantUML Export v0.1.0 Tasks

Every item uses the standing Definition of Done: acceptance criteria met,
runtime behavior verified, tests and formatting pass, public behavior is
documented, security implications are reviewed, and no unrelated changes are
included.

## Task 1: Hermetic release inputs

**Description:** Make every source/document/fixture needed by tests and release
review part of the tracked repository while retaining local artifact ignores.

**Acceptance criteria:**

- [ ] `docs/` is not ignored and required v0.1 documentation is tracked.
- [ ] Query validation uses a tracked grammar node-types fixture.
- [ ] `.codex-thread/`, build outputs, downloaded grammars, and caches remain
  ignored.

**Verification:**

- [ ] `git check-ignore` reports only intended local artifacts.
- [ ] Query-fixture and release-package tests pass from a Git archive.

**Dependencies:** None

**Files likely touched:** `.gitignore`, `docs/`, `test/fixtures/`, query tests

**Estimated scope:** Medium

## Task 2: Clean-checkout CI

**Description:** Turn the observed archive-only failure into a permanent
regression gate on all three primary operating systems.

**Acceptance criteria:**

- [ ] CI runs tests from tracked files only.
- [ ] macOS, Linux, and Windows execute native helper smoke coverage.
- [ ] Generated files are checked without mutating the checkout.

**Verification:**

- [ ] Local clean worktree/archive test passes.
- [ ] Workflow syntax and release-package checks pass.

**Dependencies:** Task 1

**Files likely touched:** `.github/workflows/ci.yml`, test/release scripts

**Estimated scope:** Small

## Task 3: Native CLI and result contracts

**Description:** Add the native crate and specify the five subcommands, stable
JSON envelopes, and process exit codes with tests first.

**Acceptance criteria:**

- [ ] `export`, `check`, `health`, `version`, and `lsp` parse as specified.
- [ ] `export` and `check` both accept repeatable additive `--include-path`.
- [ ] `--json` stdout is machine-readable and stable.
- [ ] Success, operation failure, and usage/environment failure map to 0/1/2.

**Verification:**

- [ ] Rust CLI integration tests fail before and pass after implementation.
- [ ] `cargo run ... version --json` succeeds.

**Dependencies:** Task 2

**Files likely touched:** `Cargo.toml`, `crates/plantuml-export/`

**Estimated scope:** Medium

## Task 4: Configuration contract

**Description:** Implement root/config discovery, precedence, additive include
paths, portable project settings, monotonic remote-policy restriction,
user-only trusted origins/tools, and strict validation without legacy migration.

**Acceptance criteria:**

- [ ] General precedence is CLI/Zed invocation > project > user > defaults;
  `includePaths` is additive across scopes instead of replacing earlier roots.
- [ ] Project and Zed include paths are portable and worktree-relative; nested
  configs do not cascade, while absolute include roots remain user/CLI-only.
- [ ] `remoteIncludes` accepts only `public`, `allowlist`, or `disabled`; the
  default is `public`, and project/Zed scopes can only tighten a resolved mode.
- [ ] `allowedRemoteUrls`, executable paths, JAR/Java/Graphviz paths, and
  download URLs are machine-owner user settings and cannot be granted by a
  repository or Zed worktree initialization options. Each allowed HTTP(S)
  origin contains no non-root path, credentials, query, fragment, or `;`, is
  normalized with a trailing `/`, and globally authorizes every path at that
  origin for every worktree.
- [ ] `offline = true` is sticky, prevents managed downloads, and forces remote
  includes disabled.
- [ ] The unreleased schema rejects removed `security` and boolean
  `remoteIncludes` values; no compatibility migration is implemented.
- [ ] Defaults are managed/SVG/out/public remote includes/no metadata.

**Verification:**

- [ ] Unit tests cover every source, additive path merge, restrictive policy
  collision, invalid legacy field/value, and user-only setting boundary.
- [ ] Temporary-worktree integration tests cover root and explicit config.

**Dependencies:** Task 3

**Files likely touched:** native config modules and tests

**Estimated scope:** Medium

## Task 5: Native diagnostics and LSP

**Description:** Port structural diagnostics and add real-path full syntax
checks behind a debounced, cancellable, latest-wins diagnostic-only LSP.

**Acceptance criteria:**

- [ ] In-memory diagnostics publish after about 250 ms.
- [ ] Save checks use the real URI path, 10-second timeout, standard report,
  cancellation, and stale-result suppression.
- [ ] `check --include-path` and Zed `includePaths` use the same additive include
  context as export.
- [ ] Environment failures are shown through health/status, not source ranges.

**Verification:**

- [ ] Unit tests cover structural/report parsing and line mapping.
- [ ] Stdio integration tests cover open/change/save/shutdown and stale checks.

**Dependencies:** Task 4

**Files likely touched:** native diagnostics/LSP modules and tests

**Estimated scope:** Medium

## Task 6: Renderer policy

**Description:** Implement explicit managed/binary/jar resolution, versioned
managed JRE/JAR integrity, layout requirements, and health output.

**Acceptance criteria:**

- [ ] No renderer fallback occurs across modes.
- [ ] Managed PlantUML 1.2026.6 and Temurin 21.0.11+10 downloads are locked,
  SHA-256 checked, safely extracted, and atomically installed only after
  explicit use.
- [ ] Managed mode needs no external Java/Graphviz; explicit JAR Java 17/21,
  Graphviz opt-in, and Smetana remains the default.
- [ ] Public mode maps to `INTERNET` for zero-configuration public includes;
  allowlist uses only machine-owner origins; disabled/offline maps to
  `ALLOWLIST` without an authorized origin. PlantUML's upstream INTERNET checks
  reduce SSRF exposure but are not a hard network sandbox or an absolute
  defense against DNS rebinding.
- [ ] Inherited PlantUML security/include/URL values and aggregate JVM policy
  variables (`JAVA_TOOL_OPTIONS`, `JDK_JAVA_OPTIONS`, `_JAVA_OPTIONS`) are
  cleared before every renderer invocation.
- [ ] Local include roots remain restricted in every remote policy, and source
  metadata remains disabled by default.

**Verification:**

- [ ] Unit/integration tests cover good/corrupt/concurrent/offline assets and
  prerequisite failures.
- [ ] `health --json` reports actionable component status.

**Dependencies:** Task 4

**Files likely touched:** native renderer/download/health modules and tests

**Estimated scope:** Medium

## Task 7: Discovery and output planning

**Description:** Discover standalone inputs deterministically and map each to
safe mirrored outputs without entering excluded roots or symlinked directories.

**Acceptance criteria:**

- [ ] Git/global/`.ignore` plus include/exclude globs are respected.
- [ ] Discovery is sorted, suffix-limited, symlink-safe, and excludes output,
  cache, and `.git`.
- [ ] Empty input is a no-op unless `--require-input` is set.

**Verification:**

- [ ] Temporary-worktree tests cover ignores, Unicode, traversal, symlinks,
  explicit inputs, and empty input.

**Dependencies:** Tasks 3-4

**Files likely touched:** native discovery/planning modules and tests

**Estimated scope:** Medium

## Task 8: Transactional Export Session

**Description:** Render each session in isolated platform application state,
validate outputs, commit atomically, and maintain exact source ownership in the
internal manifest while exposing only final artifacts under `out/`.

**Acceptance criteria:**

- [ ] Multi-block/newpage outputs are inventoried and type/nonzero validated.
- [ ] Default workspace failure leaves prior outputs/internal manifest intact.
- [ ] `--keep-going` commits successful sources, returns nonzero, and reports
  every failure; cleanup never removes unrelated files.
- [ ] Manifest, writer lock, render staging, rollback journal, and backups live
  under the platform `exports/<workspace-hash>` state directory and never
  appear in the configured output tree.

**Verification:**

- [ ] Fake-renderer tests cover SVG/PNG/PDF, multi-output, collisions, zero-byte
  success, timeout, rollback, stale owned output, and unrelated file safety.
- [ ] Real managed smoke succeeds when local prerequisites are available.

**Dependencies:** Tasks 6-7

**Files likely touched:** native export/manifest/validation modules and tests

**Estimated scope:** Medium

## Task 9: Zed native-helper adapter

**Description:** Download/verify/install the correct helper and launch its LSP
through the Wasm extension for all six platform contracts.

**Acceptance criteria:**

- [ ] Target mapping and exact artifact checksums are exhaustive.
- [ ] Install uses temp + verify + atomic replacement and avoids activation-time
  renderer network access.
- [ ] Language server command is the helper plus `--root <worktree> lsp`; that
  command-line root remains authoritative over initialization JSON.
- [ ] The adapter forwards
  `lsp.plantuml-lsp.initialization_options` directly without a custom root or
  settings envelope; malformed options fail closed.
- [ ] The helper accepts only portable additive `includePaths` and a restrictive
  `remoteIncludes` mode from Zed; it rejects `offline`, tools, and
  `allowedRemoteUrls`, and applies the options before renderer preparation.

**Verification:**

- [ ] Wasm build, adapter forwarding, initialization parsing, trust-boundary,
  and restrictive-merge contract tests pass.
- [ ] Dev-extension smoke launches the native LSP on the host platform.

**Dependencies:** Tasks 5-6

**Files likely touched:** `extension/src/lib.rs`, release metadata, extension tests

**Estimated scope:** Medium

## Task 10: Zed Code Actions and Node removal

**Description:** Route saved current-file SVG/PNG/PDF export through LSP Code
Actions handled by the already-running native helper, then delete the PATH
tasks/runnables, embedded JS server, Node CLI, and generated shell path after
parity.

**Acceptance criteria:**

- [ ] Code Actions expose SVG, PNG, and PDF through one advertised command.
- [ ] `workspace/executeCommand` calls the native runtime in the same helper
  process and never searches PATH for another CLI.
- [ ] Runtime source contains no Node dependency or embedded renderer shell
  implementation; static PATH tasks and runnables are absent.
- [ ] Saved-file behavior and optional standalone CLI usage are documented.

**Verification:**

- [ ] No-PATH LSP integration, release-package, Rust, and Wasm checks pass.
- [ ] Dev-extension current-file export Code Action smoke passes.

**Dependencies:** Task 9

**Files likely touched:** native LSP/runtime, removed tasks/runnables and Node
files, package/release checks

**Estimated scope:** Medium

## Task 11: Six-target GitHub release pipeline

**Description:** Build immutable helper assets, publish checksums, and attest
their provenance from a tag-triggered GitHub workflow.

**Acceptance criteria:**

- [ ] Exactly six documented binary assets are produced.
- [ ] SHA-256 manifest names every asset and the extension metadata can be
  regenerated from it.
- [ ] GitHub artifact attestations are emitted with least-privilege permissions.

**Verification:**

- [ ] Workflow/action syntax checks pass.
- [ ] Local packaging test validates names, executable/PE metadata, and checksum
  manifest parsing.

**Dependencies:** Task 10

**Files likely touched:** `.github/workflows/release.yml`, packaging scripts,
release metadata

**Estimated scope:** Medium

## Task 12: Documentation, QA, and final audit

**Description:** Align all user/developer/release documentation, run every
automatable gate, record manual limits, and map the specification to evidence.

**Acceptance criteria:**

- [ ] README covers install, CLI, config, renderer/security/network behavior,
  troubleshooting, and the no-live-preview boundary.
- [ ] Release checklist includes clean checkout, six targets, attestation,
  rollback, and dev-extension QA.
- [ ] The final audit identifies passed, failed, blocked, and manual-only items
  without claiming unrun checks passed.

**Verification:**

- [ ] Full clean-checkout test matrix and dependency/security checks run.
- [ ] Exact candidate diff/SHA is reported; execution stops before push/release
  for explicit approval.

**Dependencies:** Tasks 1-11

**Files likely touched:** README/docs/checklists/audit

**Estimated scope:** Medium
