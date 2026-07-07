import assert from "node:assert/strict";
import fs from "node:fs";
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

test("Rust extension shell starts the bundled PlantUML LSP", () => {
  const cargo = read("Cargo.toml");
  const rust = read("src/lib.rs");

  assert.match(cargo, /crate-type = \["cdylib"\]/);
  assert.match(cargo, /zed_extension_api = "0\.7\.0"/);
  assert.match(rust, /struct PlantUmlExtension/);
  assert.match(rust, /zed::register_extension!\(PlantUmlExtension\)/);
  assert.match(rust, /language_server_command/);
  assert.match(rust, /zed::node_binary_path\(\)/);
  assert.match(rust, /include_str!\("lsp-stdio\.cjs"\)/);
  assert.match(rust, /LspSettings::for_worktree\("plantuml-lsp", worktree\)/);
  assert.match(rust, /language_server_initialization_options/);
});

test("architecture documents why no explicit Zed capabilities are declared", () => {
  const architecture = read("docs/architecture.md");

  assert.match(architecture, /No explicit extension capability entry is required/);
  assert.match(architecture, /does not call `zed_extension_api::process::Command`/);
  assert.match(architecture, /zed_extension_api::download_file/);
  assert.match(architecture, /zed_extension_api::npm_install_package/);
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

test("language task templates export current file and workspace without project setup", () => {
  const packageJson = JSON.parse(read("package.json"));
  const tasks = JSON.parse(read("languages/plantuml/tasks.json"));
  const labels = tasks.map((task) => task.label);

  assert.equal(packageJson.scripts["generate:tasks"], "node scripts/generate-plantuml-tasks.mjs");

  assert.deepEqual(labels, [
    "PlantUML: export current file to PNG",
    "PlantUML: export current file to PNG and open",
    "PlantUML: export current file to SVG",
    "PlantUML: export current file to PDF",
    "PlantUML: renderer health check",
    "PlantUML: export workspace to PNG",
    "PlantUML: export workspace to SVG",
  ]);

  for (const task of tasks) {
    assert.doesNotMatch(task.command, /zed-plantuml-export\/scripts\/plantuml-export\.mjs/);
    assert.doesNotMatch(task.command, /scripts\/plantuml-export\.mjs/);
    assert.doesNotMatch(task.command, /vendor\/plantuml\.jar/);
    assert.match(task.command, /plantuml/);
    assert.match(task.command, /XDG_CACHE_HOME/);
    assert.match(task.command, /curl -fL/);
    assert.doesNotMatch(task.command, /releases\/latest/);
    assert.match(task.command, /PLANTUML_DEFAULT_VERSION="v1\.2025\.4"/);
    assert.match(task.command, /PLANTUML_DEFAULT_SHA256="26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1"/);
    assert.match(task.command, /PLANTUML_VERSION="\$\{PLANTUML_ZED_PLANTUML_VERSION:-\$PLANTUML_DEFAULT_VERSION\}"/);
    assert.match(task.command, /github\.com\/plantuml\/plantuml\/releases\/download\/\$PLANTUML_VERSION\/plantuml\.jar/);
    assert.equal(task.args, undefined);
    assert.equal(
      task.save,
      task.label.includes("workspace") ? "all" : "current",
    );
    assert.equal(task.shell.with_arguments.program, "/bin/sh");
    assert.deepEqual(task.shell.with_arguments.args, ["-c"]);

    if (task.label.includes("workspace")) {
      assert.match(task.command, /-print0/);
      assert.match(task.command, /xargs -0/);
    }
  }
});

test("PlantUML export-and-open and health check tasks are present", () => {
  const tasks = JSON.parse(read("languages/plantuml/tasks.json"));
  const exportOpen = tasks.find(
    (task) => task.label === "PlantUML: export current file to PNG and open",
  );
  const healthCheck = tasks.find(
    (task) => task.label === "PlantUML: renderer health check",
  );

  assert.ok(exportOpen);
  assert.match(exportOpen.command, /open "\$out\/\$base\.png"/);
  assert.match(exportOpen.command, /xdg-open "\$out\/\$base\.png"/);
  assert.ok(healthCheck);
  assert.match(healthCheck.command, /"\$java_bin" -version/);
  assert.match(healthCheck.command, /Graphviz/);
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

test("runnable markers connect @startuml to export tasks", () => {
  const runnables = read("languages/plantuml/runnables.scm");

  assert.match(runnables, /@startuml/);
  assert.match(runnables, /plantuml-export-png/);
});

test("release documentation covers architecture and checklist", () => {
  const readme = read("README.md");
  const architecture = read("docs/architecture.md");
  const releaseChecklist = read("docs/release-checklist.md");
  const gitignore = read(".gitignore");

  assert.match(readme, /languages\/plantuml\/tasks\.json/);
  assert.match(readme, /cache/);
  assert.match(readme, /plantuml-lsp/);
  assert.match(readme, /Server rendering is intentionally not part of this phase/);
  assert.match(architecture, /Renderer strategy/);
  assert.match(architecture, /Diagnostics/);
  assert.match(architecture, /arbitrary\s+editor-command API/);
  assert.match(releaseChecklist, /zed-industries\/extensions/);
  assert.match(gitignore, /^grammars\/$/m);
  assert.match(gitignore, /^out\/$/m);
  assert.match(gitignore, /^target\/$/m);
  assert.match(gitignore, /^tmp\/$/m);
});

test("CI workflow runs release-grade checks on supported platforms", () => {
  const workflow = read(".github/workflows/ci.yml");

  assert.match(workflow, /runs-on: \$\{\{ matrix\.os \}\}/);
  assert.match(workflow, /ubuntu-latest/);
  assert.match(workflow, /macos-latest/);
  assert.match(workflow, /windows-latest/);
  assert.match(workflow, /npm test/);
  assert.match(workflow, /cargo fmt --check/);
  assert.match(workflow, /cargo check --target wasm32-wasip1/);
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

function read(path) {
  return fs.readFileSync(new URL(path, root), "utf8");
}
