import assert from "node:assert/strict";
import fs from "node:fs";
import { test } from "node:test";

import {
  PLANTUML_TASK_HELPER_VERSION,
  buildPlantUmlTasks,
  renderPlantUmlTasksJson,
} from "../src/task-commands.mjs";

const root = new URL("../", import.meta.url);

test("buildPlantUmlTasks preserves public task labels and tags", () => {
  const tasks = buildPlantUmlTasks();

  assert.deepEqual(
    tasks.map((task) => task.label),
    [
      "PlantUML: export current file to PNG",
      "PlantUML: export current file to PNG and open",
      "PlantUML: export current file to SVG",
      "PlantUML: export current file to PDF",
      "PlantUML: renderer health check",
      "PlantUML: export workspace to PNG",
      "PlantUML: export workspace to SVG",
    ],
  );
  assert.deepEqual(tasks[0].tags, ["plantuml-export-png"]);
  assert.deepEqual(tasks[4].tags, ["plantuml-health-check"]);
  assert.equal(PLANTUML_TASK_HELPER_VERSION, 1);
});

test("language task JSON is generated from task helper module", () => {
  const committed = fs.readFileSync(
    new URL("languages/plantuml/tasks.json", root),
    "utf8",
  );

  assert.equal(committed, renderPlantUmlTasksJson());
});

test("generated task commands remain self-contained and local-only", () => {
  for (const task of buildPlantUmlTasks()) {
    assert.match(task.command, /PLANTUML_TASK_HELPER_VERSION=1/);
    assert.match(task.command, /PLANTUML_ZED_PLANTUML_VERSION/);
    assert.match(task.command, /PLANTUML_ZED_OUTPUT_DIR/);
    assert.match(task.command, /PLANTUML_ZED_PLANTUML_BIN/);
    assert.match(task.command, /PLANTUML_ZED_PLANTUML_JAR/);
    assert.match(task.command, /PLANTUML_ZED_JAVA/);
    assert.match(task.command, /PLANTUML_ZED_GRAPHVIZ_DOT/);
    assert.match(task.command, /github\.com\/plantuml\/plantuml\/releases\/download\/\$PLANTUML_VERSION\/plantuml\.jar/);
    assert.doesNotMatch(task.command, /server-url|plantuml\.com\/plantuml/);
    assert.match(task.command, /ensure_pdf_libs/);
    assert.match(task.command, /batik-all\/1\.17/);
    assert.match(task.command, /fop-transcoder-allinone\/2\.9/);
    assert.match(task.command, /verify_sha256/);
    assert.match(task.command, /sha256sum|shasum -a 256/);
    assert.match(task.command, /26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1/);
    assert.equal(task.shell.with_arguments.program, "/bin/sh");
    assert.deepEqual(task.shell.with_arguments.args, ["-c"]);
  }
});
