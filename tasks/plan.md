# Implementation Plan: PlantUML Export v0.1.0

## Overview

Replace the unreleased Node-based export/LSP path with a native Rust toolchain,
preserve the existing language assets, and prepare a reproducible six-target
GitHub prerelease. Work proceeds in five ordered stages; each checkpoint must
leave the repository buildable and verifiable.

## Architecture Decisions

- Keep the root Rust crate as the small Zed/Wasm adapter and add a separate
  native workspace crate so native-only dependencies never enter the Wasm
  extension build.
- Put configuration, discovery, diagnostics, renderer, export-session, and LSP
  behavior behind one native library used by the `plantuml-export` binary.
- Treat renderer selection as an explicit tagged mode, with managed PlantUML
  version/checksum constants compiled into the helper.
- Model export as plan -> stage -> validate -> commit, with an ownership
  manifest as the cleanup authority.
- Keep Zed language assets static and make tasks call the public CLI contract;
  remove Node and generated shell implementations after parity is verified.
- Make release artifacts immutable inputs to the Zed adapter: exact version,
  target-specific URL, and checksum.

## Dependency Graph

```text
Hermetic repository baseline
  -> CLI/config/result contracts
    -> renderer + discovery + diagnostics
      -> transactional export + native LSP
        -> Zed downloader/launcher + tasks
          -> six-target release workflow + final audit
```

## Task List

### Phase 1: Release integrity

- [ ] Task 1: Track release documents and the grammar validation fixture.
- [ ] Task 2: Add a clean-checkout regression test and make three-OS CI use the
  hermetic source surface.

### Checkpoint: Phase 1

- [ ] A Git archive/clean checkout contains all required inputs and passes its
  release-package and query-fixture checks.

### Phase 2: Native Rust toolchain

- [ ] Task 3: Define CLI, configuration, JSON result, and exit-code contracts
  with failing tests.
- [ ] Task 4: Implement configuration discovery/precedence and migration
  errors.
- [ ] Task 5: Implement structural diagnostics and diagnostic-only stdio LSP.
- [ ] Task 6: Keep the Node path available only until native parity tests pass.

### Checkpoint: Phase 2

- [ ] `plantuml-export version`, `health`, `check`, and `lsp` run natively;
  Rust unit/integration tests and the existing suite pass.

### Phase 3: Renderer and Export Session

- [ ] Task 7: Implement explicit renderer modes, managed-asset locking,
  checksum verification, Java/layout/security policy, and health reporting.
- [ ] Task 8: Implement ignore-aware discovery and deterministic output
  planning.
- [ ] Task 9: Implement staged multi-output rendering, type validation,
  manifest ownership, atomic commit, rollback, and `--keep-going`.

### Checkpoint: Phase 3

- [ ] Fake-renderer integration tests cover SVG/PNG/PDF, multi-output,
  conflicts, timeout, partial failure, rollback, and unrelated-file safety.
- [ ] A real local managed-renderer smoke test runs when prerequisites permit.

### Phase 4: Zed switch

- [ ] Task 10: Add target-aware native-helper acquisition and verified atomic
  installation to the Wasm adapter.
- [ ] Task 11: Switch language-server launch to `plantuml-export lsp` and
  replace generated shell tasks with the stable CLI contract.
- [ ] Task 12: Remove the Node runtime/server/task generator after parity and
  update extension tests.

### Checkpoint: Phase 4

- [ ] Wasm checks pass, no runtime source references Node, and task/LSP contract
  tests prove the native path.

### Phase 5: Release preparation

- [ ] Task 13: Add six-target tag workflow, checksums, and GitHub artifact
  attestations.
- [ ] Task 14: Update README, architecture, migration, security/network,
  troubleshooting, and rollback documentation.
- [ ] Task 15: Run the full clean-checkout, Rust, Wasm, release-package,
  dependency/security, and manual dev-extension QA gates.
- [ ] Task 16: Produce a requirement-by-requirement audit and exact candidate
  SHA; stop for approval before push or prerelease creation.

### Checkpoint: Complete

- [ ] Every success criterion in `docs/spec-v0.1.0.md` maps to implementation
  and current verification evidence.
- [ ] No public push, prerelease, registry submission, or external contact has
  occurred without explicit approval.

## Risks and Mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Static Zed tasks cannot directly resolve an extension-private binary | High | Verify official API/task substitutions early; define one documented launcher contract before switching tasks. |
| Cross-compiling Windows ARM64 or Linux ARM64 is fragile | High | Separate native smoke jobs from release cross-build jobs and validate artifact metadata/checksums. |
| PlantUML may exit successfully with invalid/empty output | High | Validate expected count, nonzero length, and format magic/content before commit. |
| Renderer writes partial or unexpected filenames | High | Render in isolated staging, inventory all outputs, reject conflicts, then atomically replace owned files. |
| Includes escape the intended trust boundary | High | Canonicalize allowed roots, use ALLOWLIST, disable remote includes, and test traversal/symlink cases. |
| Existing `.gitignore` contains a user-owned local backup rule | Medium | Preserve `.codex-thread/`; change only the `docs/`/fixture release-integrity behavior. |

## Open Questions

None at product level. The Zed task-to-downloaded-helper resolution is an
implementation capability check and must be resolved from official behavior
without expanding scope.
