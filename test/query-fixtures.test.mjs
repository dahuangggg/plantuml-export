import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";

const root = new URL("../", import.meta.url);

test("PlantUML query files only reference nodes from the bundled grammar", () => {
  const nodeTypes = new Set(
    JSON.parse(read("grammars/plantuml/src/node-types.json")).map(
      (node) => node.type,
    ),
  );
  const queryFiles = [
    "languages/plantuml/highlights.scm",
    "languages/plantuml/outline.scm",
    "languages/plantuml/folds.scm",
    "languages/plantuml/indents.scm",
    "languages/plantuml/brackets.scm",
  ];

  for (const file of queryFiles) {
    const query = read(file);
    for (const nodeName of extractQueryNodeNames(query)) {
      assert.ok(
        nodeTypes.has(nodeName) || nodeName === "_",
        `${file} references unknown grammar node ${nodeName}`,
      );
    }
  }
});

test("C4 fixture constructs have matching highlight coverage", () => {
  const fixture = read("test/fixtures/c4-context.puml");
  const highlights = read("languages/plantuml/highlights.scm");
  const expectedProcedures = [
    "Person",
    "System",
    "Container",
    "Component",
    "Rel",
    "LAYOUT_WITH_LEGEND",
  ];

  for (const procedure of expectedProcedures) {
    assert.match(fixture, new RegExp(`\\b${procedure}\\(`));
    assert.match(highlights, new RegExp(`"${procedure}"`));
  }
});

function extractQueryNodeNames(query) {
  const names = new Set();
  const stripped = query
    .replace(/;[^\n]*/g, "")
    .replace(/"([^"\\]|\\.)*"/g, "");
  const pattern = /\(([a-zA-Z_][\w-]*)/g;
  for (const match of stripped.matchAll(pattern)) {
    const name = match[1];
    if (!name.startsWith("#")) {
      names.add(name);
    }
  }
  return names;
}

function read(relativePath) {
  return fs.readFileSync(new URL(relativePath, root), "utf8");
}
