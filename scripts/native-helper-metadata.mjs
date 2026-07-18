export const NATIVE_HELPER_REPOSITORY = "dahuangggg/plantuml-export";

export const EXPECTED_NATIVE_HELPER_ASSETS = Object.freeze([
  helper("aarch64-apple-darwin"),
  helper("x86_64-apple-darwin"),
  helper("aarch64-unknown-linux-gnu"),
  helper("x86_64-unknown-linux-gnu"),
  helper("aarch64-pc-windows-msvc", true),
  helper("x86_64-pc-windows-msvc", true),
]);

export function buildNativeHelperMetadata(releaseTag, checksums) {
  assertReleaseTag(releaseTag);
  if (!(checksums instanceof Map)) {
    throw new TypeError("checksums must be a Map keyed by release asset name");
  }

  const expectedNames = new Set(EXPECTED_NATIVE_HELPER_ASSETS.map(({ asset }) => asset));
  if (
    checksums.size !== expectedNames.size ||
    [...checksums.keys()].some((asset) => !expectedNames.has(asset))
  ) {
    throw new Error("SHA256SUMS must contain exactly the six expected native helper assets");
  }

  const metadata = {
    schemaVersion: 1,
    repository: NATIVE_HELPER_REPOSITORY,
    status: "published",
    releaseTag,
    artifacts: EXPECTED_NATIVE_HELPER_ASSETS.map(({ target, asset }) => ({
      target,
      asset,
      url: `https://github.com/${NATIVE_HELPER_REPOSITORY}/releases/download/${releaseTag}/${asset}`,
      sha256: normalizeSha256(checksums.get(asset), asset),
    })),
  };
  validateNativeHelperMetadata(metadata);
  return metadata;
}

export function parseChecksumManifest(contents) {
  const checksums = new Map();
  for (const [index, rawLine] of contents.split(/\r?\n/).entries()) {
    const line = rawLine.trim();
    if (!line) continue;
    const match = /^([0-9a-f]{64})\s+\*?([^/\\]+)$/.exec(line);
    if (!match) {
      throw new Error(`invalid SHA256SUMS line ${index + 1}`);
    }
    const [, sha256, asset] = match;
    if (checksums.has(asset)) {
      throw new Error(`duplicate SHA256SUMS entry for ${asset}`);
    }
    checksums.set(asset, sha256);
  }
  return checksums;
}

export function validateNativeHelperMetadata(metadata) {
  if (!metadata || typeof metadata !== "object" || Array.isArray(metadata)) {
    throw new Error("native helper metadata must be an object");
  }
  assertExactKeys(metadata, [
    "schemaVersion",
    "repository",
    "status",
    "releaseTag",
    "artifacts",
  ], "native helper metadata");
  if (metadata.schemaVersion !== 1) {
    throw new Error("native helper metadata schemaVersion must be 1");
  }
  if (metadata.repository !== NATIVE_HELPER_REPOSITORY) {
    throw new Error(`native helper repository must be ${NATIVE_HELPER_REPOSITORY}`);
  }
  if (!Array.isArray(metadata.artifacts)) {
    throw new Error("native helper artifacts must be an array");
  }

  if (metadata.status === "unpublished") {
    if (metadata.releaseTag !== null || metadata.artifacts.length !== 0) {
      throw new Error("unpublished metadata must not contain a release tag or artifacts");
    }
    return metadata;
  }
  if (metadata.status !== "published") {
    throw new Error("native helper status must be unpublished or published");
  }

  assertReleaseTag(metadata.releaseTag);
  if (metadata.artifacts.length !== EXPECTED_NATIVE_HELPER_ASSETS.length) {
    throw new Error("published metadata must contain exactly six native helper artifacts");
  }

  const artifacts = new Map();
  for (const artifact of metadata.artifacts) {
    assertExactKeys(artifact, ["target", "asset", "url", "sha256"], "native helper artifact");
    if (artifacts.has(artifact.target)) {
      throw new Error(`duplicate native helper target ${artifact.target}`);
    }
    artifacts.set(artifact.target, artifact);
  }

  for (const expected of EXPECTED_NATIVE_HELPER_ASSETS) {
    const artifact = artifacts.get(expected.target);
    if (!artifact || artifact.asset !== expected.asset) {
      throw new Error(`missing native helper asset ${expected.asset}`);
    }
    const expectedUrl = `https://github.com/${NATIVE_HELPER_REPOSITORY}/releases/download/${metadata.releaseTag}/${expected.asset}`;
    if (artifact.url !== expectedUrl) {
      throw new Error(`native helper URL must be pinned to ${expectedUrl}`);
    }
    normalizeSha256(artifact.sha256, artifact.asset);
  }
  return metadata;
}

function helper(target, windows = false) {
  return Object.freeze({
    target,
    asset: `plantuml-export-${target}${windows ? ".exe" : ""}`,
  });
}

function assertReleaseTag(tag) {
  if (typeof tag !== "string" || !/^v0\.1\.0(?:-rc\.[1-9][0-9]*)?$/.test(tag)) {
    throw new Error("native helper release tag must be v0.1.0 or v0.1.0-rc.N");
  }
}

function normalizeSha256(value, asset) {
  if (
    typeof value !== "string" ||
    !/^[0-9a-f]{64}$/.test(value) ||
    /^([0-9a-f])\1{63}$/.test(value)
  ) {
    throw new Error(`invalid SHA-256 for ${asset}`);
  }
  return value;
}

function assertExactKeys(value, expectedKeys, label) {
  const actualKeys = Object.keys(value).sort();
  const sortedExpected = [...expectedKeys].sort();
  if (JSON.stringify(actualKeys) !== JSON.stringify(sortedExpected)) {
    throw new Error(`${label} has unknown or missing fields`);
  }
}
