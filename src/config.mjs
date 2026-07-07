import { PLANTUML_DEFAULT_VERSION } from "./renderer.mjs";

export const supportedLocalFormats = ["png", "svg", "pdf"];

export const defaultPlantUmlSettings = {
  autoDownloadJar: true,
  defaultFormat: "png",
  diagnosticsOnChange: true,
  graphvizDot: "dot",
  javaBinary: "java",
  outputDir: "out/plantuml",
  plantumlBinary: undefined,
  plantumlJar: undefined,
  plantumlVersion: PLANTUML_DEFAULT_VERSION,
};

export function normalizePlantUmlSettings(input = {}) {
  if (
    input.renderer === "server" ||
    input.preferServer === true ||
    input.serverUrl !== undefined
  ) {
    throw new Error("Server renderer is not supported in this local-only phase");
  }

  const settings = {
    ...defaultPlantUmlSettings,
    ...pickDefined(input, Object.keys(defaultPlantUmlSettings)),
  };

  if (!supportedLocalFormats.includes(settings.defaultFormat)) {
    throw new Error(
      `Unsupported defaultFormat "${settings.defaultFormat}". Supported formats: ${supportedLocalFormats.join(", ")}`,
    );
  }

  if (!settings.outputDir || typeof settings.outputDir !== "string") {
    throw new Error("outputDir must be a non-empty string");
  }

  if (!settings.plantumlVersion || typeof settings.plantumlVersion !== "string") {
    throw new Error("plantumlVersion must be a non-empty string");
  }

  return settings;
}

export function plantUmlSettingsToEnv(settings) {
  return omitUndefined({
    PLANTUML_ZED_AUTO_DOWNLOAD_JAR: settings.autoDownloadJar ? "1" : "0",
    PLANTUML_ZED_DEFAULT_FORMAT: settings.defaultFormat,
    PLANTUML_ZED_DIAGNOSTICS_ON_CHANGE: settings.diagnosticsOnChange ? "1" : "0",
    PLANTUML_ZED_GRAPHVIZ_DOT: settings.graphvizDot,
    PLANTUML_ZED_JAVA: settings.javaBinary,
    PLANTUML_ZED_OUTPUT_DIR: settings.outputDir,
    PLANTUML_ZED_PLANTUML_BIN: settings.plantumlBinary,
    PLANTUML_ZED_PLANTUML_JAR: settings.plantumlJar,
    PLANTUML_ZED_PLANTUML_VERSION: settings.plantumlVersion,
  });
}

function pickDefined(input, keys) {
  const result = {};
  for (const key of keys) {
    if (input[key] !== undefined) {
      result[key] = input[key];
    }
  }
  return result;
}

function omitUndefined(input) {
  return Object.fromEntries(
    Object.entries(input).filter(([, value]) => value !== undefined),
  );
}
