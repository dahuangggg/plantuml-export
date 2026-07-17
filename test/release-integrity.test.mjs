import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import { test } from "node:test";

const root = new URL("../", import.meta.url);
const requiredReleaseInputs = [
  "docs/architecture.md",
  "docs/release-checklist.md",
  "docs/spec-v0.1.0.md",
  "test/fixtures/plantuml-node-types.json",
];

test("clean checkouts contain every release and query-validation input", () => {
  for (const relativePath of requiredReleaseInputs) {
    assert.equal(
      fs.existsSync(new URL(relativePath, root)),
      true,
      `${relativePath} must be present in a clean checkout`,
    );
  }
});

test("release inputs are not ignored", () => {
  for (const relativePath of requiredReleaseInputs) {
    assert.throws(
      () =>
        execFileSync("git", ["check-ignore", "--quiet", relativePath], {
          cwd: root,
          stdio: "ignore",
        }),
      `${relativePath} must not be excluded by .gitignore`,
    );
  }
});
