import assert from "node:assert/strict";
import { test } from "node:test";

import {
  defaultPlantUmlSettings,
  normalizePlantUmlSettings,
  plantUmlSettingsToEnv,
} from "../src/config.mjs";

test("default PlantUML settings are local-only and deterministic", () => {
  assert.deepEqual(defaultPlantUmlSettings, {
    autoDownloadJar: true,
    defaultFormat: "png",
    diagnosticsOnChange: true,
    graphvizDot: "dot",
    javaBinary: "java",
    outputDir: "out/plantuml",
    plantumlBinary: undefined,
    plantumlJar: undefined,
    plantumlVersion: "v1.2025.4",
  });
});

test("normalizePlantUmlSettings merges supported user settings", () => {
  const settings = normalizePlantUmlSettings({
    autoDownloadJar: false,
    defaultFormat: "svg",
    diagnosticsOnChange: false,
    graphvizDot: "/opt/homebrew/bin/dot",
    javaBinary: "/usr/bin/java",
    outputDir: "docs/images",
    plantumlBinary: "/opt/homebrew/bin/plantuml",
    plantumlJar: "/Users/me/plantuml.jar",
    plantumlVersion: "v1.2026.1",
  });

  assert.deepEqual(settings, {
    autoDownloadJar: false,
    defaultFormat: "svg",
    diagnosticsOnChange: false,
    graphvizDot: "/opt/homebrew/bin/dot",
    javaBinary: "/usr/bin/java",
    outputDir: "docs/images",
    plantumlBinary: "/opt/homebrew/bin/plantuml",
    plantumlJar: "/Users/me/plantuml.jar",
    plantumlVersion: "v1.2026.1",
  });
});

test("normalizePlantUmlSettings rejects unsupported formats", () => {
  assert.throws(
    () => normalizePlantUmlSettings({ defaultFormat: "gif" }),
    /Unsupported defaultFormat "gif"/,
  );
});

test("normalizePlantUmlSettings rejects server renderer settings", () => {
  assert.throws(
    () => normalizePlantUmlSettings({ renderer: "server" }),
    /Server renderer is not supported/,
  );
  assert.throws(
    () => normalizePlantUmlSettings({ serverUrl: "https://example.test" }),
    /Server renderer is not supported/,
  );
});

test("plantUmlSettingsToEnv serializes settings for the LSP process", () => {
  const env = plantUmlSettingsToEnv(
    normalizePlantUmlSettings({
      defaultFormat: "pdf",
      outputDir: "build/uml",
      plantumlJar: "/tmp/plantuml.jar",
    }),
  );

  assert.deepEqual(env, {
    PLANTUML_ZED_AUTO_DOWNLOAD_JAR: "1",
    PLANTUML_ZED_DEFAULT_FORMAT: "pdf",
    PLANTUML_ZED_DIAGNOSTICS_ON_CHANGE: "1",
    PLANTUML_ZED_GRAPHVIZ_DOT: "dot",
    PLANTUML_ZED_JAVA: "java",
    PLANTUML_ZED_OUTPUT_DIR: "build/uml",
    PLANTUML_ZED_PLANTUML_JAR: "/tmp/plantuml.jar",
    PLANTUML_ZED_PLANTUML_VERSION: "v1.2025.4",
  });
});
