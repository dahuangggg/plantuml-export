import assert from "node:assert/strict";
import { test } from "node:test";

import {
  PLANTUML_DEFAULT_VERSION,
  PLANTUML_DEFAULT_SHA256,
  buildLocalRenderCommand,
  plantumlPdfDependencies,
  plantumlPdfDependencyDownloads,
  plantumlJarDownload,
  plantumlJarUrl,
  resolveLocalRenderer,
} from "../src/renderer.mjs";

test("plantumlJarUrl uses a pinned release tag", () => {
  assert.equal(PLANTUML_DEFAULT_VERSION, "v1.2025.4");
  assert.equal(
    PLANTUML_DEFAULT_SHA256,
    "26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1",
  );
  assert.equal(
    plantumlJarUrl("v1.2025.4"),
    "https://github.com/plantuml/plantuml/releases/download/v1.2025.4/plantuml.jar",
  );
  assert.deepEqual(plantumlJarDownload("v1.2025.4"), {
    url: "https://github.com/plantuml/plantuml/releases/download/v1.2025.4/plantuml.jar",
    sha256: "26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1",
  });
});

test("resolveLocalRenderer prefers configured binary", () => {
  const renderer = resolveLocalRenderer({
    settings: {
      plantumlBinary: "/usr/local/bin/plantuml",
      plantumlJar: "/tmp/plantuml.jar",
      javaBinary: "java",
      plantumlVersion: "v1.2025.4",
    },
    env: { PLANTUML_JAR: "/env/plantuml.jar" },
    exists: () => true,
    cacheDir: "/cache",
  });

  assert.deepEqual(renderer, {
    kind: "binary",
    path: "/usr/local/bin/plantuml",
  });
});

test("resolveLocalRenderer falls back to environment jar and configured jar", () => {
  assert.deepEqual(
    resolveLocalRenderer({
      settings: {
        javaBinary: "java",
        plantumlVersion: "v1.2025.4",
      },
      env: { PLANTUML_JAR: "/env/plantuml.jar" },
      exists: (path) => path === "/env/plantuml.jar",
      cacheDir: "/cache",
    }),
    {
      java: "java",
      kind: "jar",
      path: "/env/plantuml.jar",
    },
  );

  assert.deepEqual(
    resolveLocalRenderer({
      settings: {
        javaBinary: "/usr/bin/java",
        plantumlJar: "/configured/plantuml.jar",
        plantumlVersion: "v1.2025.4",
      },
      env: {},
      exists: (path) => path === "/configured/plantuml.jar",
      cacheDir: "/cache",
    }),
    {
      java: "/usr/bin/java",
      kind: "jar",
      path: "/configured/plantuml.jar",
    },
  );
});

test("resolveLocalRenderer reports cache metadata for auto-download jar", () => {
  assert.deepEqual(
    resolveLocalRenderer({
      settings: {
        autoDownloadJar: true,
        javaBinary: "java",
        plantumlVersion: "v1.2025.4",
      },
      env: {},
      exists: (path) =>
        path === "/cache/zed-plantuml/plantuml-v1.2025.4.jar",
      cacheDir: "/cache",
    }),
    {
      java: "java",
      kind: "jar",
      managed: true,
      path: "/cache/zed-plantuml/plantuml-v1.2025.4.jar",
      sha256: "26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1",
      url: "https://github.com/plantuml/plantuml/releases/download/v1.2025.4/plantuml.jar",
    },
  );

  const renderer = resolveLocalRenderer({
    settings: {
      autoDownloadJar: true,
      javaBinary: "java",
      plantumlVersion: "v1.2025.4",
    },
    env: {},
    exists: () => false,
    cacheDir: "/cache",
  });

  assert.deepEqual(renderer, {
    java: "java",
    kind: "downloadable-jar",
    managed: true,
    path: "/cache/zed-plantuml/plantuml-v1.2025.4.jar",
    sha256: "26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1",
    url: "https://github.com/plantuml/plantuml/releases/download/v1.2025.4/plantuml.jar",
  });
});

test("plantuml PDF sidecar dependencies use manifest-compatible file names", () => {
  assert.deepEqual(
    plantumlPdfDependencyDownloads({
      jarPath: "/cache/zed-plantuml/plantuml-v1.2025.4.jar",
    }).map(({ path }) => path),
    plantumlPdfDependencies.map(
      ({ fileName }) => `/cache/zed-plantuml/${fileName}`,
    ),
  );
  assert.ok(
    plantumlPdfDependencies.some(
      (dependency) =>
        dependency.fileName === "batik-all-1.7.jar" &&
        dependency.url.includes("batik-all/1.17") &&
        dependency.sha256 ===
          "174b616d93ea4dab9a6d0c23c1ab5f146123c0ee2534087931607f9ee1253191",
    ),
  );
  assert.ok(
    plantumlPdfDependencies.some(
      (dependency) =>
        dependency.fileName === "fop.jar" &&
        dependency.url.includes("fop-transcoder-allinone/2.9") &&
        dependency.sha256 ===
          "fda44e0587c751c7aecbaadf8f81641dad4173b98d8724d99068ec570d6bcda2",
    ),
  );
});

test("buildLocalRenderCommand creates PlantUML binary and jar commands", () => {
  assert.deepEqual(
    buildLocalRenderCommand({
      format: "svg",
      input: "model.puml",
      outDir: "out/plantuml",
      renderer: { kind: "binary", path: "plantuml" },
    }),
    {
      command: "plantuml",
      args: ["-failfast2", "-tsvg", "-o", "out/plantuml", "model.puml"],
    },
  );

  assert.deepEqual(
    buildLocalRenderCommand({
      format: "png",
      input: "model.puml",
      outDir: "out/plantuml",
      renderer: { kind: "jar", java: "java", path: "/cache/plantuml.jar" },
    }),
    {
      command: "java",
      args: [
        "-jar",
        "/cache/plantuml.jar",
        "-failfast2",
        "-tpng",
        "-o",
        "out/plantuml",
        "model.puml",
      ],
    },
  );
});
