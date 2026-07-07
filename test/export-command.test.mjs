import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import {
  buildRendererCommand,
  classifyRendererFailure,
  downloadFileToPath,
  ensureRendererDownloads,
  findPlantUmlFiles,
  isGeneratedOutput,
  listFreshOutputs,
  normalizeOutDir,
  parseArgs,
  requiredRendererDownloads,
  resolveRenderer,
  resolveInputs,
  supportedFormats,
  supportedRendererModes,
} from "../src/export-command.mjs";

test("parseArgs reads input, format, output directory, and jar path", () => {
  const parsed = parseArgs([
    "--format",
    "svg",
    "--out-dir",
    "out/diagrams",
    "--jar",
    "vendor/plantuml.jar",
    "docs/model.puml",
  ]);

  assert.deepEqual(parsed, {
    extraArgs: [],
    format: "svg",
    input: "docs/model.puml",
    jar: "vendor/plantuml.jar",
    java: "java",
    outDir: "out/diagrams",
    plantuml: undefined,
    plantumlVersion: "v1.2025.4",
    autoDownloadJar: true,
    renderer: "auto",
    workspace: false,
  });
});

test("parseArgs reads renderer mode and workspace mode", () => {
  const parsed = parseArgs([
    "--renderer",
    "jar",
    "--workspace",
    "--plantuml-version",
    "v1.2026.6",
    "--no-auto-download",
    "--format",
    "png",
    ".",
  ]);

  assert.equal(parsed.renderer, "jar");
  assert.equal(parsed.workspace, true);
  assert.equal(parsed.plantumlVersion, "v1.2026.6");
  assert.equal(parsed.autoDownloadJar, false);
});

test("parseArgs rejects unsupported output formats", () => {
  assert.throws(
    () => parseArgs(["--format", "gif", "model.puml"]),
    /Unsupported format "gif"/,
  );
});

test("parseArgs rejects unsupported renderer modes", () => {
  assert.throws(
    () => parseArgs(["--renderer", "magic", "model.puml"]),
    /Unsupported renderer "magic"/,
  );
});

test("parseArgs rejects server renderer options", () => {
  assert.throws(
    () => parseArgs(["--server-url", "https://plantuml.example", "model.puml"]),
    /Server renderer is not supported/,
  );
});

test("resolveRenderer prefers an explicit jar when it exists", () => {
  const renderer = resolveRenderer({
    env: {},
    fsExists: (path) => path === "vendor/plantuml.jar",
    jar: "vendor/plantuml.jar",
    plantuml: undefined,
  });

  assert.deepEqual(renderer, {
    java: "java",
    kind: "jar",
    path: "vendor/plantuml.jar",
  });
});

test("resolveRenderer honors explicit renderer mode", () => {
  assert.deepEqual(
    resolveRenderer({
      env: {},
      fsExists: () => false,
      jar: undefined,
      plantuml: "plantuml",
      renderer: "binary",
    }),
    {
      kind: "binary",
      path: "plantuml",
    },
  );

  assert.throws(
    () =>
      resolveRenderer({
        env: {},
        fsExists: () => false,
        jar: undefined,
        plantuml: undefined,
        renderer: "server",
        serverUrl: "https://plantuml.example",
      }),
    /Unsupported renderer "server"/,
  );
});

test("resolveRenderer rejects jar mode when no jar can be found", () => {
  assert.throws(
    () =>
      resolveRenderer({
        env: {},
        fsExists: () => false,
        autoDownloadJar: false,
        jar: undefined,
        plantuml: undefined,
        renderer: "jar",
      }),
    /PlantUML jar not found/,
  );
});

test("resolveRenderer rejects an explicit missing jar path", () => {
  assert.throws(
    () =>
      resolveRenderer({
        env: {},
        fsExists: () => false,
        jar: "/missing/plantuml.jar",
        plantuml: undefined,
      }),
    /PlantUML jar not found: \/missing\/plantuml\.jar/,
  );
});

test("findPlantUmlFiles discovers supported suffixes and skips generated folders", () => {
  const files = findPlantUmlFiles("/repo", {
    readdirSync: (dir) => {
      const entries = {
        "/repo": [
          dirent("docs", true),
          dirent("node_modules", true),
          dirent("out", true),
          dirent("README.md", false),
        ],
        "/repo/docs": [
          dirent("a.puml", false),
          dirent("b.plantuml", false),
          dirent("c.txt", false),
          dirent("nested", true),
        ],
        "/repo/docs/nested": [dirent("d.wsd", false), dirent("e.iuml", false)],
      };
      return entries[dir] ?? [];
    },
  });

  assert.deepEqual(files, [
    path.join("/repo", "docs", "a.puml"),
    path.join("/repo", "docs", "b.plantuml"),
    path.join("/repo", "docs", "nested", "d.wsd"),
    path.join("/repo", "docs", "nested", "e.iuml"),
  ]);
});

test("resolveInputs expands workspace mode", () => {
  const inputs = resolveInputs({
    input: "/repo",
    workspace: true,
    fsModule: {
      readdirSync: (dir) =>
        dir === "/repo" ? [dirent("model.pu", false)] : [],
    },
  });

  assert.deepEqual(inputs, [path.join("/repo", "model.pu")]);
});

test("resolveRenderer falls back to the plantuml binary", () => {
  const renderer = resolveRenderer({
    autoDownloadJar: false,
    env: {},
    fsExists: () => false,
    jar: undefined,
    plantuml: undefined,
  });

  assert.deepEqual(renderer, {
    kind: "binary",
    path: "plantuml",
  });
});

test("buildRendererCommand creates java -jar invocation for jar rendering", () => {
  const command = buildRendererCommand({
    extraArgs: [],
    format: "png",
    input: "model.puml",
    java: "java",
    outDir: "out/plantuml",
    renderer: { kind: "jar", path: "vendor/plantuml.jar" },
  });

  assert.deepEqual(command, {
    command: "java",
    args: [
      "-jar",
      "vendor/plantuml.jar",
      "-failfast2",
      "-tpng",
      "-o",
      "out/plantuml",
      "model.puml",
    ],
  });
});

test("normalizeOutDir resolves relative output directories from the task cwd", () => {
  assert.equal(
    normalizeOutDir("out/plantuml", "/project"),
    "/project/out/plantuml",
  );
  assert.equal(
    normalizeOutDir("out\\plantuml", "C:\\repo", path.win32),
    "C:\\repo\\out\\plantuml",
  );
  assert.equal(
    normalizeOutDir("D:\\diagrams", "C:\\repo", path.win32),
    "D:\\diagrams",
  );
});

test("buildRendererCommand creates plantuml binary invocation", () => {
  const command = buildRendererCommand({
    extraArgs: [],
    format: "svg",
    input: "model.puml",
    java: "java",
    outDir: "out/plantuml",
    renderer: { kind: "binary", path: "plantuml" },
  });

  assert.deepEqual(command, {
    command: "plantuml",
    args: ["-failfast2", "-tsvg", "-o", "out/plantuml", "model.puml"],
  });
});

test("requiredRendererDownloads downloads managed jar and PDF sidecar dependencies", () => {
  const downloads = requiredRendererDownloads({
    format: "pdf",
    fsExists: () => false,
    renderer: {
      java: "java",
      kind: "downloadable-jar",
      managed: true,
      path: "/cache/zed-plantuml/plantuml-v1.2025.4.jar",
      sha256: "26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1",
      url: "https://github.com/plantuml/plantuml/releases/download/v1.2025.4/plantuml.jar",
    },
  });

  assert.equal(downloads.length, 7);
  assert.deepEqual(downloads[0], {
    path: "/cache/zed-plantuml/plantuml-v1.2025.4.jar",
    sha256: "26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1",
    url: "https://github.com/plantuml/plantuml/releases/download/v1.2025.4/plantuml.jar",
  });
  assert.ok(downloads.some((download) => download.path.endsWith("batik-all-1.7.jar")));
  assert.ok(downloads.some((download) => download.path.endsWith("fop.jar")));
});

test("requiredRendererDownloads redownloads corrupt managed cached jars", () => {
  const downloads = requiredRendererDownloads({
    format: "png",
    fileSha256: () =>
      "0000000000000000000000000000000000000000000000000000000000000000",
    fsExists: () => true,
    renderer: {
      java: "java",
      kind: "jar",
      managed: true,
      path: "/cache/zed-plantuml/plantuml-v1.2025.4.jar",
      sha256: "26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1",
      url: "https://github.com/plantuml/plantuml/releases/download/v1.2025.4/plantuml.jar",
    },
  });

  assert.deepEqual(downloads, [
    {
      path: "/cache/zed-plantuml/plantuml-v1.2025.4.jar",
      sha256: "26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1",
      url: "https://github.com/plantuml/plantuml/releases/download/v1.2025.4/plantuml.jar",
    },
  ]);
});

test("requiredRendererDownloads does not mutate user-provided jars", () => {
  const downloads = requiredRendererDownloads({
    format: "pdf",
    fsExists: () => false,
    renderer: {
      java: "java",
      kind: "jar",
      path: "/user/plantuml.jar",
    },
  });

  assert.deepEqual(downloads, []);
});

test("ensureRendererDownloads delegates each missing asset to the downloader", async () => {
  const calls = [];

  await ensureRendererDownloads({
    downloads: [
      { path: "/cache/plantuml.jar", url: "https://example.test/plantuml.jar" },
      { path: "/cache/fop.jar", url: "https://example.test/fop.jar" },
    ],
    downloadFile: async (url, path) => calls.push([url, path]),
  });

  assert.deepEqual(calls, [
    ["https://example.test/plantuml.jar", "/cache/plantuml.jar"],
    ["https://example.test/fop.jar", "/cache/fop.jar"],
  ]);
});

test("downloadFileToPath rejects checksum mismatches and leaves target untouched", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "puml-download-test-"));
  const target = path.join(dir, "asset.jar");

  await assert.rejects(
    () =>
      downloadFileToPath(
        "https://example.test/asset.jar",
        target,
        {
          expectedSha256:
            "0000000000000000000000000000000000000000000000000000000000000000",
        },
        async () => ({
          ok: true,
          status: 200,
          arrayBuffer: async () => Buffer.from("not-the-expected-content"),
        }),
      ),
    /Checksum mismatch/,
  );

  assert.equal(fs.existsSync(target), false);
});

test("classifyRendererFailure explains common local renderer failures", () => {
  assert.match(
    classifyRendererFailure({
      command: "java",
      error: Object.assign(new Error("spawn java ENOENT"), { code: "ENOENT" }),
    }),
    /Renderer command not found: java/,
  );
  assert.match(
    classifyRendererFailure({
      command: "java",
      output:
        "java.lang.UnsupportedClassVersionError: this version only recognizes class file versions up to 61.0",
    }),
    /Java runtime is too old/,
  );
  assert.match(
    classifyRendererFailure({
      command: "java",
      format: "pdf",
      output:
        "java.lang.ClassNotFoundException: org.apache.batik.apps.rasterizer.SVGConverter",
    }),
    /PDF export needs PlantUML Batik\/FOP sidecar libraries/,
  );
  assert.match(
    classifyRendererFailure({
      command: "plantuml",
      output: "Cannot find Graphviz. You should try dot -V.",
    }),
    /Graphviz dot was not found/,
  );
});

test("supportedFormats lists practical PlantUML export targets", () => {
  assert.deepEqual(supportedFormats, ["png", "svg", "pdf"]);
});

test("supportedRendererModes lists local renderers only", () => {
  assert.deepEqual(supportedRendererModes, ["auto", "binary", "jar"]);
});

test("isGeneratedOutput matches single and multi-page PlantUML outputs", () => {
  assert.equal(isGeneratedOutput("sample.png", "examples/sample.puml", "png"), true);
  assert.equal(
    isGeneratedOutput("sample_001.png", "examples/sample.puml", "png"),
    true,
  );
  assert.equal(isGeneratedOutput("sample.svg", "examples/sample.puml", "png"), false);
  assert.equal(isGeneratedOutput("other.png", "examples/sample.puml", "png"), false);
});

function dirent(name, isDirectory) {
  return {
    name,
    isDirectory: () => isDirectory,
    isFile: () => !isDirectory,
  };
}

test("listFreshOutputs ignores zero-byte renderer artifacts", () => {
  const files = listFreshOutputs({
    format: "pdf",
    outDir: "/project/out",
    since: 1000,
    fsModule: {
      existsSync: () => true,
      readdirSync: () => ["empty.pdf", "valid.pdf", "sample.png"],
      statSync: (filePath) => ({
        mtimeMs: 2000,
        size: filePath.endsWith("valid.pdf") ? 128 : 0,
      }),
    },
  });

  assert.deepEqual(files, ["/project/out/valid.pdf"]);
});
