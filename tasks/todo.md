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
- [ ] `--json` stdout is machine-readable and stable.
- [ ] Success, operation failure, and usage/environment failure map to 0/1/2.

**Verification:**

- [ ] Rust CLI integration tests fail before and pass after implementation.
- [ ] `cargo run ... version --json` succeeds.

**Dependencies:** Task 2

**Files likely touched:** `Cargo.toml`, `crates/plantuml-export-cli/`

**Estimated scope:** Medium

## Task 4: Configuration contract

**Description:** Implement root/config discovery, precedence, portable project
settings, user-only executable settings, and explicit migration errors.

**Acceptance criteria:**

- [ ] Precedence is CLI > project > user > defaults.
- [ ] Nested configs do not cascade and untrusted project paths cannot select
  executables/download URLs.
- [ ] Defaults are managed/SVG/out/plantuml/ALLOWLIST/no metadata.

**Verification:**

- [ ] Unit tests cover every source and precedence collision.
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
- [ ] Environment failures are shown through health/status, not source ranges.

**Verification:**

- [ ] Unit tests cover structural/report parsing and line mapping.
- [ ] Stdio integration tests cover open/change/save/shutdown and stale checks.

**Dependencies:** Task 4

**Files likely touched:** native diagnostics/LSP modules and tests

**Estimated scope:** Medium

## Task 6: Renderer policy

**Description:** Implement explicit managed/binary/jar resolution, versioned
managed download integrity, Java/layout requirements, and health output.

**Acceptance criteria:**

- [ ] No renderer fallback occurs across modes.
- [ ] Managed 1.2026.6 download is locked, SHA-256 checked, and atomically
  installed only after explicit use.
- [ ] Java 17/21, Graphviz/Smetana, ALLOWLIST, remote-include, offline, and
  metadata policies are enforced.

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

**Description:** Render each session in isolated staging, validate outputs,
commit atomically, and maintain exact source ownership in the manifest.

**Acceptance criteria:**

- [ ] Multi-block/newpage outputs are inventoried and type/nonzero validated.
- [ ] Default workspace failure leaves prior outputs/manifest intact.
- [ ] `--keep-going` commits successful sources, returns nonzero, and reports
  every failure; cleanup never removes unrelated files.

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
- [ ] Language server command is the helper plus `lsp`, with worktree settings.

**Verification:**

- [ ] Wasm build and adapter contract tests pass.
- [ ] Dev-extension smoke launches the native LSP on the host platform.

**Dependencies:** Tasks 5-6

**Files likely touched:** `src/lib.rs`, release metadata, extension tests

**Estimated scope:** Medium

## Task 10: Zed tasks and Node removal

**Description:** Route current/workspace tasks and runnables through the public
CLI, then delete the old embedded JS server, Node CLI, and giant generated shell
path after parity.

**Acceptance criteria:**

- [ ] Tasks preserve practical labels while defaulting export to SVG.
- [ ] Runtime source and production tasks contain no Node dependency or embedded
  renderer shell implementation.
- [ ] Saved-file behavior and health task are documented.

**Verification:**

- [ ] Task contract, release-package, Rust, and Wasm checks pass.
- [ ] Dev-extension current-file/workspace export smoke passes.

**Dependencies:** Task 9

**Files likely touched:** `languages/plantuml/tasks.json`, runnables, Node files,
package/release checks

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
  migration, troubleshooting, and no-live-preview boundary.
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
