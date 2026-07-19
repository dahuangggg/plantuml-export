import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

const workflow = fs.readFileSync(
  new URL("../.github/workflows/release.yml", import.meta.url),
  "utf8",
).replaceAll("\r\n", "\n");
const smoke = fs
  .readFileSync(
    new URL("../scripts/smoke-native-helper.sh", import.meta.url),
    "utf8",
  )
  .replaceAll("\r\n", "\n");

const targets = [
  ["macos-15", "aarch64-apple-darwin", "plantuml-export-aarch64-apple-darwin"],
  ["macos-15-intel", "x86_64-apple-darwin", "plantuml-export-x86_64-apple-darwin"],
  [
    "ubuntu-24.04-arm",
    "aarch64-unknown-linux-gnu",
    "plantuml-export-aarch64-unknown-linux-gnu",
  ],
  [
    "ubuntu-24.04",
    "x86_64-unknown-linux-gnu",
    "plantuml-export-x86_64-unknown-linux-gnu",
  ],
  [
    "windows-11-arm",
    "aarch64-pc-windows-msvc",
    "plantuml-export-aarch64-pc-windows-msvc.exe",
  ],
  [
    "windows-2025",
    "x86_64-pc-windows-msvc",
    "plantuml-export-x86_64-pc-windows-msvc.exe",
  ],
];

test("release workflow is tag-only and contains the exact six native targets", () => {
  assert.match(workflow, /tags:\n\s+- "v0\.1\.0-rc\.1"/);
  assert.doesNotMatch(workflow, /workflow_dispatch/);
  assert.match(workflow, /\$\{tag\}" != "v0\.1\.0-rc\.1"/);

  for (const [runner, target, asset] of targets) {
    assert.match(workflow, new RegExp(`runner: ${escapeRegex(runner)}`));
    assert.match(workflow, new RegExp(`target: ${escapeRegex(target)}`));
    assert.equal(
      workflow.match(new RegExp(`asset: ${escapeRegex(asset)}`, "g"))?.length,
      1,
      `${asset} must appear exactly once as a matrix asset`,
    );
  }
  assert.equal(workflow.match(/^\s+asset: plantuml-export-/gm)?.length, 6);
});

test("tag gate requires pristine unpublished helper metadata", () => {
  assert.match(
    workflow,
    /\.status == "unpublished"[\s\S]+\.releaseTag == null[\s\S]+\.artifacts == \[\][\s\S]+release\/native-helper-release\.json/,
  );
  const archiveGate = workflow.slice(
    workflow.indexOf("git archive HEAD"),
    workflow.indexOf("Preserve the reviewed release notes"),
  );
  assert.ok(
    archiveGate.indexOf("npm run check:release-package") <
      archiveGate.indexOf("cargo test --locked"),
    "the no-Git package scan must run before builds create target/",
  );
});

test("tag gate scans secrets and audits the locked Rust dependency graph", () => {
  assert.match(workflow, /fetch-depth: 0/);
  assert.match(
    workflow,
    /gitleaks\/gitleaks-action@e0c47f4f8be36e29cdc102c57e68cb5cbf0e8d1e/,
  );
  assert.match(workflow, /GITLEAKS_VERSION: "8\.30\.1"/);
  assert.match(
    workflow,
    /gitleaks git --redact --no-banner --log-opts="--all" \./,
  );
  assert.match(
    workflow,
    /actions\/setup-node@48b55a011bda9f5d6aeb4c2d9c7362e8dae4041e/,
  );
  assert.match(workflow, /node-version: "24"/);
  assert.match(workflow, /package-manager-cache: false/);
  assert.match(workflow, /cargo install cargo-audit --version 0\.22\.2 --locked/);
  assert.match(workflow, /cargo audit --deny warnings --file Cargo\.lock/);
});

test("release workflow pins actions and gives write permissions only to publish", () => {
  const uses = [...workflow.matchAll(/^\s+uses:\s+([^\s]+)$/gm)].map(
    (match) => match[1],
  );
  assert.ok(uses.length > 0);
  for (const action of uses) {
    assert.match(action, /^[^@]+@[0-9a-f]{40}$/, `${action} is not immutable`);
  }

  const publish = workflow.slice(workflow.indexOf("  publish:"));
  assert.doesNotMatch(publish, /actions\/checkout/);
  assert.match(publish, /contents: write/);
  assert.match(publish, /id-token: write/);
  assert.match(publish, /attestations: write/);
  assert.match(publish, /artifact-metadata: write/);
  assert.match(publish, /GH_REPO: \$\{\{ github\.repository \}\}/);
  assert.match(publish, /subject-checksums: dist\/SHA256SUMS/);
  assert.match(publish, /subject-path: dist\/SHA256SUMS/);
});

test("release stays draft until every asset and attestation step succeeds", () => {
  assert.match(workflow, /gh release create[\s\S]+--draft/);
  assert.match(workflow, /name: release-notes/);
  assert.match(workflow, /path: release\/notes-v0\.1\.0-rc\.1\.md/);
  assert.match(
    workflow,
    /gh release create[\s\S]+--notes-file release\/notes-v0\.1\.0-rc\.1\.md/,
  );
  assert.doesNotMatch(
    workflow,
    /--generate-notes/,
    "the public release body must be exactly the reviewed notes artifact",
  );
  assert.match(
    workflow,
    /gh release edit[\s\S]+--draft=false[\s\S]+--prerelease[\s\S]+--latest=false/,
  );
  assert.ok(
    workflow.indexOf("subject-checksums") < workflow.indexOf("gh release create"),
  );
});

test("every host-native release binary runs the managed export smoke", () => {
  assert.match(
    workflow,
    /bash scripts\/smoke-native-helper\.sh[\s\S]+dist\/\$\{\{ matrix\.asset \}\}[\s\S]+graphviz-class\.puml[\s\S]+RUNNER_TEMP/,
  );
  assert.ok(
    workflow.indexOf("Smoke managed SVG, PNG, and PDF exports") <
      workflow.indexOf("Upload native binary"),
  );
  assert.ok(
    smoke.indexOf('--json check') < smoke.indexOf('--json health'),
    "the first managed check must install prerequisites before read-only health",
  );
  assert.ok(
    smoke.indexOf('--json health') < smoke.indexOf('--json export'),
    "the smoke must validate installed prerequisites before exporting",
  );
});

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
