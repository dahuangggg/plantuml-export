import fs from "node:fs";
import crypto from "node:crypto";
import path from "node:path";

import { defaultPlantUmlSettings, normalizePlantUmlSettings } from "./config.mjs";
import {
  buildLocalRenderCommand,
  plantumlPdfDependencyDownloads,
  resolveLocalRenderer,
} from "./renderer.mjs";

export const supportedFormats = ["png", "svg", "pdf"];
export const supportedRendererModes = ["auto", "binary", "jar"];
export const plantumlSuffixes = [".puml", ".plantuml", ".pu", ".wsd", ".iuml"];

export function parseArgs(argv) {
  const options = {
    extraArgs: [],
    format: "png",
    input: undefined,
    jar: undefined,
    java: "java",
    outDir: "out/plantuml",
    plantuml: undefined,
    plantumlVersion: defaultPlantUmlSettings.plantumlVersion,
    autoDownloadJar: true,
    renderer: "auto",
    workspace: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];

    switch (arg) {
      case "--format":
      case "-t":
        options.format = requireValue(argv, (index += 1), arg);
        break;
      case "--out-dir":
      case "-o":
        options.outDir = requireValue(argv, (index += 1), arg);
        break;
      case "--jar":
        options.jar = requireValue(argv, (index += 1), arg);
        break;
      case "--java":
        options.java = requireValue(argv, (index += 1), arg);
        break;
      case "--plantuml":
        options.plantuml = requireValue(argv, (index += 1), arg);
        break;
      case "--plantuml-version":
        options.plantumlVersion = requireValue(argv, (index += 1), arg);
        break;
      case "--no-auto-download":
        options.autoDownloadJar = false;
        break;
      case "--server-url":
        throw new Error("Server renderer is not supported in this local-only phase");
      case "--renderer":
        options.renderer = requireValue(argv, (index += 1), arg);
        break;
      case "--workspace":
        options.workspace = true;
        break;
      case "--":
        options.extraArgs.push(...argv.slice(index + 1));
        index = argv.length;
        break;
      default:
        if (arg.startsWith("-")) {
          options.extraArgs.push(arg);
        } else if (!options.input) {
          options.input = arg;
        } else {
          options.extraArgs.push(arg);
        }
    }
  }

  if (!options.input) {
    throw new Error("Missing PlantUML input file");
  }

  if (!supportedFormats.includes(options.format)) {
    throw new Error(
      `Unsupported format "${options.format}". Supported formats: ${supportedFormats.join(", ")}`,
    );
  }

  if (!supportedRendererModes.includes(options.renderer)) {
    throw new Error(
      `Unsupported renderer "${options.renderer}". Supported renderers: ${supportedRendererModes.join(", ")}`,
    );
  }

  return options;
}

export function resolveRenderer({
  cacheDir,
  env = process.env,
  fsExists = fs.existsSync,
  autoDownloadJar = true,
  jar,
  java = "java",
  plantuml,
  plantumlVersion = defaultPlantUmlSettings.plantumlVersion,
  renderer = "auto",
}) {
  if (!supportedRendererModes.includes(renderer)) {
    throw new Error(
      `Unsupported renderer "${renderer}". Supported renderers: ${supportedRendererModes.join(", ")}`,
    );
  }

  if (renderer === "binary") {
    return { kind: "binary", path: plantuml || env.PLANTUML_BIN || "plantuml" };
  }

  if (jar && !fsExists(jar)) {
    throw new Error(`PlantUML jar not found: ${jar}`);
  }

  const settings = normalizePlantUmlSettings({
    autoDownloadJar,
    javaBinary: java,
    plantumlBinary: renderer === "auto" ? plantuml || env.PLANTUML_BIN : undefined,
    plantumlJar: jar,
    plantumlVersion,
  });

  const localRenderer = resolveLocalRenderer({
    cacheDir,
    env,
    exists: fsExists,
    settings,
  });

  if (renderer === "jar" && localRenderer.kind === "binary") {
    throw new Error(
      "PlantUML jar not found. Pass --jar, set PLANTUML_JAR, or enable automatic jar download.",
    );
  }

  return localRenderer;
}

export function buildRendererCommand({
  extraArgs = [],
  format,
  input,
  java = "java",
  outDir,
  renderer,
}) {
  if (renderer.kind === "server") {
    throw new Error("Unsupported renderer kind: server");
  }

  return buildLocalRenderCommand({
    extraArgs,
    format,
    input,
    outDir,
    renderer: renderer.kind === "jar" ? { ...renderer, java } : renderer,
  });
}

export function requiredRendererDownloads({
  fileSha256 = sha256File,
  format,
  fsExists = fs.existsSync,
  renderer,
}) {
  const downloads = [];

  if (isManagedDownloadableRenderer(renderer) && shouldDownloadAsset({
    asset: renderer,
    fileSha256,
    fsExists,
  })) {
    downloads.push({
      path: renderer.path,
      sha256: renderer.sha256,
      url: renderer.url,
    });
  }

  if (
    format === "pdf" &&
    renderer.managed === true &&
    (renderer.kind === "jar" || renderer.kind === "downloadable-jar")
  ) {
    for (const dependency of plantumlPdfDependencyDownloads({
      jarPath: renderer.path,
    })) {
      if (shouldDownloadAsset({ asset: dependency, fileSha256, fsExists })) {
        downloads.push({
          path: dependency.path,
          sha256: dependency.sha256,
          url: dependency.url,
        });
      }
    }
  }

  return downloads;
}

export async function ensureRendererDownloads({
  downloadFile = downloadFileToPath,
  downloads,
  log = () => {},
}) {
  for (const download of downloads) {
    log(`Downloading ${download.url} to ${download.path}`);
    await downloadFile(download.url, download.path, {
      expectedSha256: download.sha256,
    });
  }
}

export async function downloadFileToPath(
  url,
  filePath,
  optionsOrFetch = {},
  maybeFetchImpl,
) {
  const options =
    typeof optionsOrFetch === "function" ? {} : optionsOrFetch;
  const fetchImpl =
    typeof optionsOrFetch === "function"
      ? optionsOrFetch
      : maybeFetchImpl ?? globalThis.fetch;

  if (typeof fetchImpl !== "function") {
    throw new Error("Automatic PlantUML downloads require a Node.js runtime with fetch support.");
  }

  const response = await fetchImpl(url);
  if (!response.ok) {
    throw new Error(`Download failed for ${url}: HTTP ${response.status}`);
  }

  const buffer = Buffer.from(await response.arrayBuffer());
  if (options.expectedSha256) {
    const actualSha256 = sha256Buffer(buffer);
    if (actualSha256 !== options.expectedSha256) {
      throw new Error(
        `Checksum mismatch for ${url}. Expected ${options.expectedSha256}, got ${actualSha256}.`,
      );
    }
  }

  fs.mkdirSync(path.dirname(filePath), { recursive: true });

  const temporaryPath = `${filePath}.tmp-${process.pid}`;
  fs.writeFileSync(temporaryPath, buffer);
  fs.renameSync(temporaryPath, filePath);
}

export function findPlantUmlFiles(root, fsModule = fs) {
  const files = [];

  visit(root);
  return files.sort();

  function visit(dir) {
    for (const entry of fsModule.readdirSync(dir, { withFileTypes: true })) {
      const fullPath = path.join(dir, entry.name);

      if (entry.isDirectory()) {
        if (shouldSkipDirectory(entry.name, fullPath)) {
          continue;
        }
        visit(fullPath);
        continue;
      }

      if (entry.isFile() && plantumlSuffixes.includes(path.extname(entry.name))) {
        files.push(fullPath);
      }
    }
  }
}

export function resolveInputs({
  fsModule = fs,
  input,
  workspace = false,
}) {
  if (!workspace) {
    return [input];
  }

  return findPlantUmlFiles(input, fsModule);
}

export function normalizeOutDir(outDir, cwd = process.cwd(), pathModule = path) {
  if (pathModule.isAbsolute(outDir)) {
    return outDir;
  }

  return pathModule.resolve(cwd, outDir);
}

export function classifyRendererFailure({
  command,
  error,
  format,
  output = "",
  status,
}) {
  if (error?.code === "ENOENT") {
    return `Renderer command not found: ${command}. Install it or configure the matching PlantUML setting.`;
  }

  if (/UnsupportedClassVersionError|class file versions? up to/i.test(output)) {
    return "Java runtime is too old for the selected PlantUML jar. Install a newer Java runtime or choose a PlantUML version compatible with your Java.";
  }

  if (/ClassNotFoundException: org\.apache\.batik|SVGConverter|NoClassDefFoundError: org\/apache\/batik/i.test(output)) {
    return "PDF export needs PlantUML Batik/FOP sidecar libraries. Managed cached jars download them automatically; for a user-provided jar, place PlantUML's PDF libraries next to that jar.";
  }

  if (/Cannot find Graphviz|Dot executable|dot -V|No dot executable/i.test(output)) {
    return "Graphviz dot was not found. Install Graphviz or make sure dot is available on PATH.";
  }

  return `PlantUML renderer exited with status ${status ?? "unknown"}.`;
}

function shouldDownloadAsset({ asset, fileSha256, fsExists }) {
  if (!fsExists(asset.path)) {
    return true;
  }

  return Boolean(asset.sha256 && fileSha256(asset.path) !== asset.sha256);
}

function isManagedDownloadableRenderer(renderer) {
  return (
    renderer.managed === true &&
    typeof renderer.url === "string" &&
    (renderer.kind === "downloadable-jar" || renderer.kind === "jar")
  );
}

export function sha256File(filePath) {
  return sha256Buffer(fs.readFileSync(filePath));
}

function sha256Buffer(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

export function isGeneratedOutput(fileName, input, format) {
  const stem = path.basename(input, path.extname(input));
  const escapedStem = stem.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const escapedFormat = format.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(`^${escapedStem}(?:_\\d{3})?\\.${escapedFormat}$`);

  return pattern.test(fileName);
}

export function listFreshOutputs({
  format,
  fsModule = fs,
  outDir,
  since,
}) {
  if (!fsModule.existsSync(outDir)) {
    return [];
  }

  return fsModule
    .readdirSync(outDir)
    .filter((fileName) => fileName.endsWith(`.${format}`))
    .map((fileName) => path.join(outDir, fileName))
    .filter((filePath) => {
      const stat = fsModule.statSync(filePath);
      return stat.mtimeMs >= since - 1000 && stat.size > 0;
    });
}

function shouldSkipDirectory(name, fullPath) {
  return (
    name === ".git" ||
    name === "node_modules" ||
    name === "vendor" ||
    fullPath.includes(`${path.sep}out${path.sep}plantuml`)
  );
}

function requireValue(argv, index, flag) {
  const value = argv[index];
  if (!value || value.startsWith("--")) {
    throw new Error(`Missing value for ${flag}`);
  }
  return value;
}
