import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

const root = new URL("../", import.meta.url);

test("extension manifest registers PlantUML grammar and snippets", () => {
  const manifest = read("extension.toml");

  assert.match(manifest, /id = "plantuml-export"/);
  assert.match(manifest, /name = "PlantUML Export"/);
  assert.match(manifest, /snippets = \["\.\/snippets\/plantuml\.json"\]/);
  assert.match(manifest, /\[grammars\.plantuml\]/);
  assert.match(manifest, /tree-sitter-plantuml/);
});

test("extension manifest registers a PlantUML language server", () => {
  const manifest = read("extension.toml");

  assert.match(manifest, /\[language_servers\.plantuml-lsp\]/);
  assert.match(manifest, /name = "PlantUML LSP"/);
  assert.match(manifest, /languages = \["PlantUML"\]/);
});

test("Rust extension shell starts the native PlantUML helper without Node", () => {
  const cargo = read("Cargo.toml");
  const rust = read("extension/src/lib.rs");

  assert.match(cargo, /crate-type = \["cdylib"\]/);
  assert.match(cargo, /path = "extension\/src\/lib\.rs"/);
  assert.match(cargo, /zed_extension_api = "0\.7\.0"/);
  assert.match(cargo, /sha2 = "0\.10"/);
  assert.match(rust, /struct PlantUmlExtension/);
  assert.match(rust, /zed::register_extension!\(PlantUmlExtension\)/);
  assert.match(rust, /language_server_command/);
  assert.match(rust, /worktree\.which\(NATIVE_HELPER_NAME\)/);
  assert.match(rust, /resolve_managed_helper/);
  assert.match(rust, /zed::download_file/);
  assert.match(rust, /zed::make_file_executable/);
  assert.match(rust, /MAX_NATIVE_HELPER_BYTES/);
  assert.match(
    rust,
    /include_str!\("\.\.\/\.\.\/release\/native-helper-release\.json"\)/,
  );
  assert.match(rust, /"lsp"\.to_string\(\)/);
  assert.doesNotMatch(rust, /node_binary_path|lsp-stdio\.cjs/);
  assert.match(rust, /LspSettings::for_worktree\("plantuml-lsp", worktree\)/);
  assert.match(rust, /language_server_initialization_options/);
  assert.match(rust, /\.initialization_options/);
  assert.doesNotMatch(rust, /\.and_then\(\|settings\| settings\.settings\)/);
});

test("architecture documents plugin-managed exports through the native LSP helper", () => {
  const architecture = read("docs/architecture.md");

  assert.match(architecture, /`worktree\.which\("plantuml-export"\)`/);
  assert.match(architecture, /private extension working directory/);
  assert.match(architecture, /`textDocument\/codeAction`/);
  assert.match(architecture, /`workspace\/executeCommand`/);
  assert.match(architecture, /same native helper\/LSP process/);
  assert.doesNotMatch(architecture, /tasks therefore require `plantuml-export` on `PATH`/);
});

test("PlantUML language config recognizes common file suffixes", () => {
  const config = read("languages/plantuml/config.toml");

  assert.match(config, /name = "PlantUML"/);
  for (const suffix of ["wsd", "pu", "puml", "plantuml", "iuml"]) {
    assert.match(config, new RegExp(`"${suffix}"`));
  }
});

test("every language directory has a config file", () => {
  const languagesDir = new URL("languages/", root);
  const languageNames = fs
    .readdirSync(languagesDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name);

  for (const name of languageNames) {
    assert.equal(
      fs.existsSync(new URL(`languages/${name}/config.toml`, root)),
      true,
      `${name} language directory is missing config.toml`,
    );
  }
});

test("PlantUML queries avoid grammar-conflict node types", () => {
  const highlights = read("languages/plantuml/highlights.scm");
  const brackets = read("languages/plantuml/brackets.scm");

  assert.doesNotMatch(highlights, /\(comment\)/);
  assert.doesNotMatch(brackets, /"\\""/);
});

test("PlantUML highlights include function-style C4 and stdlib constructs", () => {
  const highlights = read("languages/plantuml/highlights.scm");

  assert.match(highlights, /#any-of\? @type/);
  assert.match(highlights, /"Person"/);
  assert.match(highlights, /"System"/);
  assert.match(highlights, /"Container"/);
  assert.match(highlights, /"Rel"/);
  assert.match(highlights, /"LAYOUT_WITH_LEGEND"/);
  assert.match(highlights, /@keyword.import/);
});

test("PlantUML editor queries include outline, folds, and indents", () => {
  const outline = read("languages/plantuml/outline.scm");
  const folds = read("languages/plantuml/folds.scm");
  const indents = read("languages/plantuml/indents.scm");

  assert.match(outline, /procedure_identifier/);
  assert.match(outline, /@name/);
  assert.match(folds, /\(block\) @fold/);
  assert.match(indents, /@indent/);
});

test("PlantUML snippets cover common diagram types", () => {
  const snippets = JSON.parse(read("snippets/plantuml.json"));

  for (const name of [
    "Sequence Diagram",
    "Use Case Diagram",
    "State Diagram",
    "Activity Diagram",
    "Class Diagram",
    "Component Diagram",
    "Deployment Diagram",
    "C4 Context Diagram",
    "Object Diagram",
    "Timing Diagram",
    "Gantt Chart",
    "Mindmap",
    "Work Breakdown Structure",
    "Entity Relationship Diagram",
    "Sequence Alt/Loop Block",
  ]) {
    assert.ok(snippets[name], `${name} snippet is missing`);
  }
});

test("Zed export actions use the managed LSP helper and never a PATH task", () => {
  const lsp = read("crates/plantuml-export/src/lsp.rs");
  const sample = read("examples/sample.puml");

  assert.equal(fs.existsSync(new URL("languages/plantuml/tasks.json", root)), false);
  assert.equal(fs.existsSync(new URL("languages/plantuml/runnables.scm", root)), false);
  assert.match(lsp, /EXPORT_COMMAND: &str = "plantuml-export\.export"/);
  assert.match(lsp, /"textDocument\/codeAction"/);
  assert.match(lsp, /"workspace\/executeCommand"/);
  assert.match(lsp, /run_export_file/);
  assert.match(sample, /Code Action/);
  assert.match(sample, /same helper process/);
  assert.doesNotMatch(sample, /Zed Task|run export task/);
});

test("PlantUML works with Zed generic Markdown fenced-code injection", () => {
  const readme = read("README.md");

  assert.equal(fs.existsSync(new URL("languages/markdown/config.toml", root)), false);
  assert.equal(fs.existsSync(new URL("languages/markdown/injections.scm", root)), false);
  assert.match(readme, /```plantuml/);
  assert.match(readme, /built-in\s+Markdown fenced-code injection/);
});

test("community plugin does not require parent workspace tasks", () => {
  assert.equal(fs.existsSync(new URL("../.zed/tasks.json", root)), false);
  assert.equal(fs.existsSync(new URL(".zed/tasks.json", root)), false);
});

test("release documentation covers architecture and checklist", () => {
  const readme = read("README.md");
  const architecture = read("docs/architecture.md");
  const releaseChecklist = read("docs/release-checklist.md");
  const gitignore = read(".gitignore");

  assert.match(readme, /Code Action/);
  assert.match(readme, /cache/);
  assert.match(readme, /plantuml-lsp/);
  assert.match(readme, /Server rendering is intentionally not part of this phase/);
  assert.match(architecture, /Renderer strategy/);
  assert.match(architecture, /Diagnostics/);
  assert.match(architecture, /workspace\/executeCommand/);
  assert.match(releaseChecklist, /GitHub Release Checklist/);
  assert.match(releaseChecklist, /does not submit to `zed-industries\/extensions`/);
  assert.match(gitignore, /^grammars\/$/m);
  assert.match(gitignore, /^out\/$/m);
  assert.match(gitignore, /^target\/$/m);
  assert.match(gitignore, /^tmp\/$/m);
});

test("English and Chinese READMEs keep the public toolchain contract aligned", () => {
  const readme = read("README.md");
  const chineseReadme = read("README.zh-CN.md");
  const stableTerms = [
    "v0.1.0-rc.1",
    "plantuml-export export",
    "plantuml-export check",
    "plantuml-export health",
    "plantuml-export version",
    "plantuml-export lsp",
    "plantuml-export.toml",
    "Application Support/plantuml-export",
    "89948f14c93756c7a3fb7b69078ff37e8489fd79dd430c582b931e2f65358690",
    "ALLOWLIST",
    "workspace/executeCommand",
    "SHA256SUMS",
  ];

  assert.match(readme, /\[简体中文\]\(README\.zh-CN\.md\)/);
  assert.match(chineseReadme, /\[English\]\(README\.md\)/);
  for (const term of stableTerms) {
    assert.equal(readme.includes(term), true, `README.md is missing ${term}`);
    assert.equal(
      chineseReadme.includes(term),
      true,
      `README.zh-CN.md is missing ${term}`,
    );
  }

  assert.equal(
    chineseReadme.match(/^## /gm)?.length,
    readme.match(/^## /gm)?.length,
    "README heading counts must stay aligned",
  );
  assert.equal(
    chineseReadme.match(/^```/gm)?.length,
    readme.match(/^```/gm)?.length,
    "README code-fence counts must stay aligned",
  );
});

test("Zed dev-install surfaces use wasm32-wasip2 exclusively", () => {
  const activeSurfaces = [
    "rust-toolchain.toml",
    ".github/workflows/ci.yml",
    ".github/workflows/release.yml",
    "README.md",
    "README.zh-CN.md",
    "docs/release-checklist.md",
  ];

  for (const path of activeSurfaces) {
    const contents = read(path);

    assert.match(
      contents,
      /\bwasm32-wasip2\b/,
      `${path} must use Zed's wasm32-wasip2 dev-install target`,
    );
    assert.doesNotMatch(
      contents,
      /\bwasm32-wasip1\b/,
      `${path} must not reference the obsolete wasm32-wasip1 target`,
    );
  }

  for (const workflowPath of [
    ".github/workflows/ci.yml",
    ".github/workflows/release.yml",
  ]) {
    const workflow = read(workflowPath);
    assert.match(
      workflow,
      /cargo build --locked --target wasm32-wasip2 --target-dir target/,
      `${workflowPath} must build the same root default member as Zed dev-install`,
    );
    assert.doesNotMatch(
      workflow,
      /cargo check --locked -p plantuml-export-zed --target wasm32-wasip2/,
      `${workflowPath} must not replace Zed's root build with a package-only check`,
    );
  }
});

test("CI workflow runs release-grade checks on supported platforms", () => {
  const workflow = read(".github/workflows/ci.yml");

  const uses = [...workflow.matchAll(/^\s+uses:\s+([^\s]+)$/gm)].map(
    (match) => match[1],
  );
  assert.ok(uses.length > 0);
  for (const action of uses) {
    assert.match(action, /^[^@]+@[0-9a-f]{40}$/, `${action} is not immutable`);
  }

  assert.match(workflow, /runs-on: \$\{\{ matrix\.runner \}\}/);
  assert.match(workflow, /macos-15-intel/);
  assert.match(workflow, /ubuntu-24\.04-arm/);
  assert.match(workflow, /windows-11-arm/);
  assert.equal(workflow.match(/^\s+- runner:/gm)?.length, 6);
  assert.match(workflow, /npm test/);
  assert.equal(
    workflow.match(
      /actions\/setup-node@48b55a011bda9f5d6aeb4c2d9c7362e8dae4041e/g,
    )?.length,
    2,
  );
  assert.equal(workflow.match(/node-version: "24"/g)?.length, 2);
  assert.equal(workflow.match(/package-manager-cache: false/g)?.length, 2);
  assert.match(workflow, /cargo fmt --all -- --check/);
  assert.match(workflow, /cargo test --locked --workspace --all-targets/);
  assert.match(workflow, /cargo clippy --locked --workspace --all-targets -- -D warnings/);
  assert.match(workflow, /cargo build --locked --target wasm32-wasip2 --target-dir target/);
  assert.match(workflow, /fetch-depth: 0/);
  assert.match(workflow, /cargo install cargo-audit --version 0\.22\.2 --locked/);
  assert.match(workflow, /cargo audit --deny warnings --file Cargo\.lock/);
  assert.match(
    workflow,
    /gitleaks\/gitleaks-action@e0c47f4f8be36e29cdc102c57e68cb5cbf0e8d1e/,
  );
  assert.match(workflow, /GITLEAKS_VERSION: "8\.30\.1"/);
  assert.match(
    workflow,
    /gitleaks git --redact --no-banner --log-opts="--all" \./,
  );
  assert.match(workflow, /npm run check:release-package/);
});

test("release package hygiene blocks generated artifacts and bundled renderers", () => {
  const packageJson = JSON.parse(read("package.json"));
  const gitignore = read(".gitignore");
  const checker = read("scripts/check-release-package.mjs");

  assert.equal(
    packageJson.scripts["check:release-package"],
    "node scripts/check-release-package.mjs",
  );
  for (const pattern of [
    /^extension\.wasm$/m,
    /^target\/$/m,
    /^out\/$/m,
    /^vendor\/$/m,
    /^tmp\/$/m,
    /^grammars\/$/m,
  ]) {
    assert.match(gitignore, pattern);
  }
  assert.match(checker, /extension\.wasm/);
  assert.match(checker, /vendor\/plantuml\.jar/);
  assert.match(checker, /Release package check passed/);
});

test("release package fallback rejects forbidden artifacts without Git metadata", () => {
  const temporaryRoot = fs.mkdtempSync(
    path.join(os.tmpdir(), "plantuml-release-package-"),
  );

  try {
    const scriptsDirectory = path.join(temporaryRoot, "scripts");
    fs.mkdirSync(scriptsDirectory);
    fs.copyFileSync(
      new URL("scripts/check-release-package.mjs", root),
      path.join(scriptsDirectory, "check-release-package.mjs"),
    );
    fs.mkdirSync(path.join(temporaryRoot, "out"));
    fs.writeFileSync(path.join(temporaryRoot, "out", "diagram.svg"), "<svg />");
    fs.writeFileSync(path.join(temporaryRoot, "extension.wasm"), "not wasm");
    fs.writeFileSync(path.join(temporaryRoot, "renderer#copy.jar"), "not a jar");
    if (process.platform !== "win32") {
      fs.symlinkSync(
        path.join(temporaryRoot, "scripts", "check-release-package.mjs"),
        path.join(temporaryRoot, "linked-renderer.jar"),
      );
    }

    const result = spawnSync(
      process.execPath,
      [path.join(scriptsDirectory, "check-release-package.mjs")],
      { encoding: "utf8" },
    );

    assert.equal(result.status, 1, result.stderr || result.stdout);
    assert.match(
      result.stderr,
      /Generated or local-only artifact is tracked: out/,
    );
    assert.match(
      result.stderr,
      /Generated or local-only artifact is tracked: extension\.wasm/,
    );
    assert.match(
      result.stderr,
      /Renderer jar must not be bundled in the extension source: renderer#copy\.jar/,
    );
    if (process.platform !== "win32") {
      assert.match(
        result.stderr,
        /Symbolic links must not be bundled in the extension source: linked-renderer\.jar/,
      );
    }
  } finally {
    fs.rmSync(temporaryRoot, { recursive: true, force: true });
  }
});

function read(path) {
  return fs.readFileSync(new URL(path, root), "utf8");
}
