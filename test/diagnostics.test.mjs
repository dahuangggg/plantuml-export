import assert from "node:assert/strict";
import { test } from "node:test";

import {
  parsePlantUmlOutputDiagnostics,
  structuralDiagnostics,
} from "../src/diagnostics.mjs";

test("parsePlantUmlOutputDiagnostics maps PlantUML line errors to LSP ranges", () => {
  const diagnostics = parsePlantUmlOutputDiagnostics({
    output:
      "Error line 3 in file: docs/bad.puml\nSome diagram description contains errors\n",
  });

  assert.deepEqual(diagnostics, [
    {
      range: {
        start: { line: 2, character: 0 },
        end: { line: 2, character: 1 },
      },
      severity: 1,
      source: "plantuml",
      message: "Some diagram description contains errors",
    },
  ]);
});

test("parsePlantUmlOutputDiagnostics ignores successful output", () => {
  assert.deepEqual(
    parsePlantUmlOutputDiagnostics({ output: "File generation OK\n" }),
    [],
  );
});

test("structuralDiagnostics reports a missing end directive", () => {
  const diagnostics = structuralDiagnostics("@startuml\nAlice -> Bob : hi\n");

  assert.deepEqual(diagnostics, [
    {
      range: {
        start: { line: 0, character: 0 },
        end: { line: 0, character: 9 },
      },
      severity: 1,
      source: "plantuml",
      message: "Missing matching @end directive.",
    },
  ]);
});

test("structuralDiagnostics reports non-empty files without start directive", () => {
  const diagnostics = structuralDiagnostics("Alice -> Bob : hi\n");

  assert.deepEqual(diagnostics, [
    {
      range: {
        start: { line: 0, character: 0 },
        end: { line: 0, character: 17 },
      },
      severity: 2,
      source: "plantuml",
      message: "PlantUML files should start with an @start directive.",
    },
  ]);
});

test("structuralDiagnostics reports diagrams with no body", () => {
  const diagnostics = structuralDiagnostics("@startuml\n@enduml\n");

  assert.deepEqual(diagnostics, [
    {
      range: {
        start: { line: 0, character: 0 },
        end: { line: 1, character: 7 },
      },
      severity: 2,
      source: "plantuml",
      message: "PlantUML diagram has no body.",
    },
  ]);
});
