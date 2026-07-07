import path from "node:path";

export const PLANTUML_DEFAULT_VERSION = "v1.2025.4";
export const PLANTUML_DEFAULT_SHA256 =
  "26518e14a3a04100cd76c0d96cab2d1171f36152215edd9790a28d20268200c1";

export const plantumlPdfDependencies = [
  {
    fileName: "batik-all-1.7.jar",
    sha256: "174b616d93ea4dab9a6d0c23c1ab5f146123c0ee2534087931607f9ee1253191",
    url: "https://repo1.maven.org/maven2/org/apache/xmlgraphics/batik-all/1.17/batik-all-1.17.jar",
  },
  {
    fileName: "fop.jar",
    sha256: "fda44e0587c751c7aecbaadf8f81641dad4173b98d8724d99068ec570d6bcda2",
    url: "https://repo1.maven.org/maven2/org/apache/xmlgraphics/fop-transcoder-allinone/2.9/fop-transcoder-allinone-2.9.jar",
  },
  {
    fileName: "xmlgraphics-commons-1.4.jar",
    sha256: "2ebd333ab2a624514793c336e3af086608673286fe37ba1e639e0ac3e1b58be2",
    url: "https://repo1.maven.org/maven2/org/apache/xmlgraphics/xmlgraphics-commons/2.9/xmlgraphics-commons-2.9.jar",
  },
  {
    fileName: "commons-io-1.3.1.jar",
    sha256: "a58af12ee1b68cfd2ebb0c27caef164f084381a00ec81a48cc275fd7ea54e154",
    url: "https://repo1.maven.org/maven2/commons-io/commons-io/2.15.1/commons-io-2.15.1.jar",
  },
  {
    fileName: "commons-logging-1.0.4.jar",
    sha256: "daddea1ea0be0f56978ab3006b8ac92834afeefbd9b7e4e6316fca57df0fa636",
    url: "https://repo1.maven.org/maven2/commons-logging/commons-logging/1.2/commons-logging-1.2.jar",
  },
  {
    fileName: "xml-apis-ext-1.3.04.jar",
    sha256: "d0b4887dc34d57de49074a58affad439a013d0baffa1a8034f8ef2a5ea191646",
    url: "https://repo1.maven.org/maven2/xml-apis/xml-apis-ext/1.3.04/xml-apis-ext-1.3.04.jar",
  },
];

export function plantumlJarUrl(version) {
  return `https://github.com/plantuml/plantuml/releases/download/${version}/plantuml.jar`;
}

export function plantumlJarDownload(version) {
  return {
    url: plantumlJarUrl(version),
    ...(version === PLANTUML_DEFAULT_VERSION
      ? { sha256: PLANTUML_DEFAULT_SHA256 }
      : {}),
  };
}

export function plantumlJarCachePath({ cacheDir, version }) {
  return path.join(cacheDir, "zed-plantuml", `plantuml-${version}.jar`);
}

export function resolveLocalRenderer({
  cacheDir = defaultCacheDir(),
  env = process.env,
  exists,
  settings,
}) {
  if (settings.plantumlBinary) {
    return { kind: "binary", path: settings.plantumlBinary };
  }

  const jarCandidates = [
    env.PLANTUML_JAR,
    settings.plantumlJar,
  ].filter(Boolean);

  const jar = jarCandidates.find((candidate) => exists(candidate));
  if (jar) {
    return {
      java: settings.javaBinary,
      kind: "jar",
      path: jar,
    };
  }

  const cachedJar = plantumlJarCachePath({
    cacheDir,
    version: settings.plantumlVersion,
  });

  if (exists(cachedJar)) {
    return {
      java: settings.javaBinary,
      kind: "jar",
      managed: true,
      path: cachedJar,
      ...plantumlJarDownload(settings.plantumlVersion),
    };
  }

  if (settings.autoDownloadJar) {
    const download = plantumlJarDownload(settings.plantumlVersion);
    return {
      java: settings.javaBinary,
      kind: "downloadable-jar",
      managed: true,
      path: cachedJar,
      ...download,
    };
  }

  return { kind: "binary", path: "plantuml" };
}

export function plantumlPdfDependencyDownloads({ jarPath }) {
  const directory = path.dirname(jarPath);

  return plantumlPdfDependencies.map((dependency) => ({
    ...dependency,
    path: path.join(directory, dependency.fileName),
  }));
}

export function buildLocalRenderCommand({
  extraArgs = [],
  format,
  input,
  outDir,
  renderer,
}) {
  const renderArgs = [
    "-failfast2",
    `-t${format}`,
    "-o",
    outDir,
    ...extraArgs,
    input,
  ];

  if (renderer.kind === "jar" || renderer.kind === "downloadable-jar") {
    return {
      command: renderer.java,
      args: ["-jar", renderer.path, ...renderArgs],
    };
  }

  return {
    command: renderer.path,
    args: renderArgs,
  };
}

function defaultCacheDir() {
  if (process.platform === "darwin" && !process.env.XDG_CACHE_HOME) {
    return path.join(process.env.HOME ?? ".", "Library", "Caches");
  }

  return process.env.XDG_CACHE_HOME ?? path.join(process.env.HOME ?? ".", ".cache");
}
