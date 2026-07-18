# GitHub Release Checklist

This checklist publishes only the independent PlantUML Export toolchain on
GitHub. It does not submit to `zed-industries/extensions`, contact another
extension maintainer, publish a package registry entry, or create a public PR.

## 1. Candidate source gate

- [ ] `crates/plantuml-export` reports exactly `0.1.0-rc.1`.
- [ ] `extension.toml`, the root package, and static assets describe v0.1.0.
- [ ] `rust-toolchain.toml` pins the build toolchain.
- [ ] `release/native-helper-release.json` is still explicitly `unpublished`;
      no checksum placeholder is present.
- [ ] `.codex-thread/`, `target/`, `out/`, `grammars/`, caches, downloaded JARs,
      and generated binaries are untracked.
- [ ] A fresh `git archive HEAD` contains every document and fixture needed by
      tests.

Run from tracked source:

```bash
cargo fmt --all -- --check
cargo test --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --target wasm32-wasip2 --target-dir target
cargo install cargo-audit --version 0.22.2 --locked
cargo audit --deny warnings --file Cargo.lock
npm test
npm run check:release-package
git diff --check
```

- [ ] `cargo-audit 0.22.2` reports no denied RustSec advisory for the committed
      `Cargo.lock`.
- [ ] Gitleaks `8.30.1` scans both the exact candidate source tree and complete
      Git history with no finding; CI and the tag gate use the same pinned
      versions.

- [ ] With external Java and Graphviz absent from `PATH`,
      `check test/fixtures/graphviz-class.puml` succeeds and real managed SVG,
      PNG, and PDF exports pass through the pinned Temurin JRE/JAR and Smetana.
- [ ] `test/fixtures/output-name-escape.puml` produces only
      `out/test/fixtures/output-name-escape.svg`; no
      `plantuml-export-escape-sentinel*` path appears anywhere in the worktree.
- [ ] `find out -type f` lists final SVG/PNG/PDF artifacts only: no ownership
      manifest, lock, render staging, journal, backup, or other intermediate
      state is present below `out`.
- [ ] Export bookkeeping is instead rooted at the documented platform state
      path under `exports/<workspace-hash>` and is isolated from another
      workspace/output pair.
- [ ] The ownership manifest has `schemaVersion: 2`; every output entry contains
      a normalized `path` and lowercase 64-character `sha256`, and a recreated
      `out` tree with user replacement bytes fails closed instead of overwriting.
- [ ] A state/output cross-volume fixture fails before renderer execution or
      output mutation; the transaction never falls back to copying.
- [ ] `check test/fixtures/syntax-error.puml --json` exits `1` with
      `error.kind == "operation"`, `error.code == "syntax_errors"`, and
      `data.failures[0].diagnostics[0].line == 2`.
- [ ] Managed PDF records Temurin `21.0.11+10` in the internal state manifest;
      explicit JAR mode still rejects Java 17 for PDF.
- [ ] `health --json` reports managed Java 21, Smetana/Graphviz not required,
      and the pinned renderer state.
- [ ] Every target-specific Temurin URL and checksum matches the official
      `jdk-21.0.11+10` release, and archive traversal/link/size-limit tests pass.

## 2. Zed dev-extension gate

- [ ] While metadata is unpublished, a local development helper starts
      `plantuml-export lsp`; document that this is not the released user flow.
- [ ] All five supported suffixes select PlantUML.
- [ ] Queries, snippets, outline, folds, indents, and brackets load without Zed
      log errors.
- [ ] Saved PlantUML files show SVG/PNG/PDF Code Actions through the inline
      lightning button and `Cmd-.` / `Ctrl-.`.
- [ ] A Code Action exports Unicode and nested paths from saved disk content.
- [ ] An unsaved buffer shows a save-first error and never exports stale disk
      content.
- [ ] No extension task or runnable invokes `plantuml-export` from `PATH`.
- [ ] A broken source publishes a diagnostic; an environment failure does not
      create a fake source diagnostic.
- [ ] With the helper removed from PATH and metadata unpublished, LSP startup
      fails with the intentional actionable message.

The final consumer gate is plugin-only: `plantuml-export` must be absent from
`PATH`; the extension-managed helper must perform the export itself.

## 3. Approval before GitHub push

- [ ] Record the exact candidate commit SHA and complete local evidence.
- [ ] Confirm the diff contains only the accepted v0.1 scope.
- [ ] Obtain explicit approval before pushing the candidate branch/commit.
- [ ] After push, require the six-target CI matrix to pass at that exact SHA.

Do not infer tag approval from branch-push approval.

## 4. Approval before prerelease tag

- [ ] Enable GitHub private vulnerability reporting and verify
      `gh api repos/dahuangggg/plantuml-export/private-vulnerability-reporting --jq .enabled`
      returns `true`, so [SECURITY.md](../SECURITY.md) has a working private
      report channel.
- [ ] Enable GitHub immutable releases in repository Settings before creating
      the tag; the current repository setting is a release blocker until this
      is done.
- [ ] Verify the setting with
      `gh api repos/dahuangggg/plantuml-export/immutable-releases --jq .enabled`
      and require the result to be `true`.
- [ ] Obtain separate explicit approval for `v0.1.0-rc.1`.
- [ ] Create the tag only at the verified main-ancestor SHA.
- [ ] Do not move or overwrite an existing tag.

The tag workflow must produce exactly:

```text
plantuml-export-aarch64-apple-darwin
plantuml-export-x86_64-apple-darwin
plantuml-export-aarch64-unknown-linux-gnu
plantuml-export-x86_64-unknown-linux-gnu
plantuml-export-aarch64-pc-windows-msvc.exe
plantuml-export-x86_64-pc-windows-msvc.exe
SHA256SUMS
```

- [ ] Gate job validates tag/version/main ancestry from a Git archive.
- [ ] Every binary is built and tested on its matching host-native runner.
- [ ] Every host-native binary provisions the pinned managed JRE/JAR and passes
      `health`, `check`, and real SVG/PNG/PDF export before upload.
- [ ] The publish job downloads exactly six binary artifacts plus the reviewed
      release-notes artifact and runs no repository code.
- [ ] The publish job uses the reviewed, tag-tracked
      `release/notes-v0.1.0-rc.1.md` body without checking out or executing
      repository code.
- [ ] `sha256sum --check SHA256SUMS` passes.
- [ ] Binary and checksum-manifest attestations succeed before the draft is
      made public as a prerelease.

## 5. Consumer verification

Download into an empty directory and verify:

```bash
gh release download v0.1.0-rc.1 \
  --repo dahuangggg/plantuml-export \
  --dir verify
gh release verify v0.1.0-rc.1 \
  --repo dahuangggg/plantuml-export
gh release verify-asset v0.1.0-rc.1 verify/SHA256SUMS \
  --repo dahuangggg/plantuml-export
gh attestation verify verify/SHA256SUMS \
  --repo dahuangggg/plantuml-export \
  --signer-workflow dahuangggg/plantuml-export/.github/workflows/release.yml \
  --source-ref refs/tags/v0.1.0-rc.1
(cd verify && sha256sum --check SHA256SUMS)
chmod +x verify/plantuml-export-*
gh attestation verify verify/plantuml-export-x86_64-unknown-linux-gnu \
  --repo dahuangggg/plantuml-export \
  --signer-workflow dahuangggg/plantuml-export/.github/workflows/release.yml \
  --source-ref refs/tags/v0.1.0-rc.1
```

- [ ] Run the downloaded host binary's `version`, `health`, `check`, and SVG
      export commands in a clean consumer directory.
- [ ] Confirm the release page contains no source/JAR/cache artifacts beyond
      GitHub's automatic source archives.

## 6. Publish Zed helper metadata as a follow-up

Only after step 5:

```bash
npm run generate:helper-metadata -- \
  --tag v0.1.0-rc.1 \
  --checksums verify/SHA256SUMS \
  --output release/native-helper-release.json
npm test
cargo test --locked -p plantuml-export-zed
cargo build --locked --target wasm32-wasip2 --target-dir target
```

- [ ] Review all six generated URLs and SHA-256 values.
- [ ] In the same follow-up, change README.md, README.zh-CN.md, and CHANGELOG.md
      from source-candidate/`unpublished` instructions to the verified
      `published` extension flow. The primary install path must no longer require
      `cargo install` or a standalone CLI.
- [ ] Commit the generated metadata; do not amend or retag RC1.
- [ ] Verify the Zed dev extension can download, hash, install, and launch the
      helper with PATH intentionally absent.
- [ ] In a clean Zed profile with no standalone CLI installed, install only the
      extension, with Java and Graphviz absent, and export saved SVG, PNG, and
      PDF through Code Actions after the automatic first-use download.
- [ ] Confirm `workspace/executeCommand` runs inside that helper and no task,
      shell wrapper, or second `plantuml-export` process is involved.

## Rollback

- If the tag workflow fails before publication, keep/delete the draft and fix a
  new commit; never move the tag.
- If a public RC asset or attestation is wrong, mark/delete the prerelease as
  appropriate, leave metadata unpublished, and issue a new `rc.N` tag.
- If published adapter metadata is wrong, revert that metadata commit so the
  adapter fails closed; do not weaken checksum validation.
