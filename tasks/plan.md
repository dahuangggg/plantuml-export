# Implementation Plan: PlantUML Export v0.1.0

## Overview

Build the export/LSP path as a native Rust toolchain, preserve the language
assets, and prepare a reproducible six-target GitHub prerelease. Work proceeds
in five ordered stages; each checkpoint must leave the repository buildable and
verifiable.

## Architecture Decisions

- Keep the root Rust crate as the small Zed/Wasm adapter and add a separate
  native workspace crate so native-only dependencies never enter the Wasm
  extension build.
- Put configuration, discovery, diagnostics, renderer, export-session, and LSP
  behavior behind one native library used by the `plantuml-export` binary.
- Treat renderer selection as an explicit tagged mode, with managed PlantUML
  and Temurin JRE version/checksum constants compiled into the helper; default
  to Smetana so installed extensions need no external Java or Graphviz.
- Model export as plan -> stage -> validate -> commit, with an ownership
  manifest as the cleanup authority. Keep the manifest, writer lock, render
  staging, rollback journal, and backups in platform application state keyed by
  workspace/output hash, so the default `out/` tree contains final artifacts
  only.
- Keep Zed language assets static and expose export through native LSP Code
  Actions handled by the same helper process; remove PATH tasks, Node, and
  generated shell implementations after parity is verified.
- Make release artifacts immutable inputs to the Zed adapter: exact version,
  target-specific URL, and checksum.
- Make public HTTP(S) includes work out of the box through PlantUML `INTERNET`.
  Treat `public < allowlist < disabled` as a monotonic restriction: project
  configuration and Zed worktree initialization options may tighten the user
  policy but cannot loosen it or add machine-owner `allowedRemoteUrls`. Each
  user-configured HTTP(S) origin authorizes all paths there across all
  workspaces, so it is deliberately a global machine setting.
- Forward Zed `initialization_options` directly to a strict native schema that
  accepts only portable additive `includePaths` and `remoteIncludes`; keep
  `--root`, `offline`, tools, and trusted origins outside that untrusted
  surface.
- Use a clean first-release configuration schema with no compatibility layer:
  remove the old `security` field and boolean `remoteIncludes` rather than
  migrating them.

## Dependency Graph

```text
Hermetic repository baseline
  -> CLI/config/result contracts
    -> renderer + discovery + diagnostics
      -> transactional export + native LSP
        -> Zed downloader/launcher + LSP Code Actions
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
- [ ] Task 4: Implement configuration discovery, additive include paths,
  monotonic remote-policy restriction, and trust-scoped validation without
  legacy migration.
- [ ] Task 5: Implement structural diagnostics and diagnostic-only stdio LSP.
- [ ] Task 6: Keep the Node path available only until native parity tests pass.

### Checkpoint: Phase 2

- [ ] `plantuml-export version`, `health`, `check`, and `lsp` run natively;
  Rust unit/integration tests and the existing suite pass.

### Phase 3: Renderer and Export Session

- [ ] Task 7: Implement explicit renderer modes, managed-asset locking,
  checksum verification, Java/layout policy, INTERNET/ALLOWLIST remote policy,
  offline override, and health reporting.
- [ ] Task 8: Implement ignore-aware discovery and deterministic output
  planning.
- [ ] Task 9: Implement staged multi-output rendering in private application
  state, type validation, manifest ownership, atomic commit, rollback, and
  `--keep-going`; default final outputs go directly under `out/`.

### Checkpoint: Phase 3

- [ ] Fake-renderer integration tests cover SVG/PNG/PDF, multi-output,
  conflicts, timeout, partial failure, rollback, and unrelated-file safety.
- [ ] A real local managed-renderer smoke test provisions its pinned runtime
  and validates SVG/PNG/PDF without external Java or Graphviz prerequisites.

### Phase 4: Zed switch

- [ ] Task 10: Add target-aware native-helper acquisition and verified atomic
  installation to the Wasm adapter.
- [ ] Task 11: Switch language-server launch to `plantuml-export lsp`, add
  strict Zed `initialization_options`, SVG/PNG/PDF Code Actions plus
  `workspace/executeCommand`, and remove static PATH tasks and runnables.
- [ ] Task 12: Remove the Node runtime/server/task generator after parity and
  update extension tests.

### Checkpoint: Phase 4

- [ ] Wasm checks pass, no runtime source references Node or a PATH export task,
  and LSP contract tests prove the plugin-managed native path.

### Phase 5: Release preparation

- [ ] Task 13: Add six-target tag workflow, checksums, and GitHub artifact
  attestations.
- [ ] Task 14: Update README, architecture, security/network, troubleshooting,
  and rollback documentation.
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
| Static Zed tasks cannot resolve an extension-private binary | High | Use LSP Code Actions and `workspace/executeCommand` so the already-running managed helper performs export in-process. |
| ARM64 and x86_64 behavior can diverge across operating systems | High | Build, test, and smoke every release binary on its matching host-native runner, then validate the exact artifact set and checksums. |
| PlantUML may exit successfully with invalid/empty output | High | Validate expected count, nonzero length, and format magic/content before commit. |
| Renderer writes partial or unexpected filenames | High | Render in isolated staging under private application state, inventory all outputs, reject conflicts, then atomically replace owned files; expose only final artifacts under `out/`. |
| Local or remote includes escape the intended trust boundary | High | Canonicalize local roots; use INTERNET only for public-network mode while documenting that its upstream checks reduce SSRF risk but are not a hard sandbox; keep whole-origin grants user-only and machine-global; let project/Zed settings only tighten to allowlist/disabled; make offline fail closed; clear PlantUML and aggregate JVM policy variables; test traversal, symlink, origin-policy, DNS-rebinding assumptions, and inherited-environment cases. |
| Managed JRE archives traverse paths or expand unexpectedly | High | Pin official URLs/checksums, reject links/traversal/special files, cap compressed/expanded sizes and entry count, then install atomically. |
| Existing `.gitignore` contains a user-owned local backup rule | Medium | Preserve `.codex-thread/`; change only the `docs/`/fixture release-integrity behavior. |

## Open Questions

None at product level. Official Zed behavior confirms that Code Actions can
route `workspace/executeCommand` back to the downloaded helper; static tasks
cannot resolve the extension-private helper and are therefore excluded.
