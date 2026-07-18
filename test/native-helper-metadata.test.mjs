import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import {
  EXPECTED_NATIVE_HELPER_ASSETS,
  buildNativeHelperMetadata,
  parseChecksumManifest,
  validateNativeHelperMetadata,
} from "../scripts/native-helper-metadata.mjs";

const root = new URL("../", import.meta.url);

test("checked-in native helper metadata matches its declared release state", () => {
  const metadata = JSON.parse(
    fs.readFileSync(new URL("release/native-helper-release.json", root), "utf8"),
  );

  assertCheckedInMetadata(metadata);
});

test("checked-in metadata contract accepts the generated published follow-up", () => {
  const metadata = buildNativeHelperMetadata("v0.1.0-rc.1", validChecksums());

  assert.doesNotThrow(() => assertCheckedInMetadata(metadata));
});

test("metadata builder requires all six native helper checksums", () => {
  const incomplete = new Map([
    [EXPECTED_NATIVE_HELPER_ASSETS[0].asset, "1".repeat(64)],
  ]);

  assert.throws(
    () => buildNativeHelperMetadata("v0.1.0-rc.1", incomplete),
    /exactly the six expected native helper assets/,
  );
});

test("metadata builder rejects placeholder checksums", () => {
  const checksums = validChecksums();
  checksums.set(EXPECTED_NATIVE_HELPER_ASSETS[0].asset, "0".repeat(64));

  assert.throws(
    () => buildNativeHelperMetadata("v0.1.0-rc.1", checksums),
    /invalid SHA-256/,
  );
});

test("checksum manifest rejects non-canonical uppercase digests", () => {
  assert.throws(
    () =>
      parseChecksumManifest(
        `${"A".repeat(64)}  ${EXPECTED_NATIVE_HELPER_ASSETS[0].asset}\n`,
      ),
    /invalid SHA256SUMS line/,
  );
});

test("checksum manifest rejects duplicate and path-bearing asset names", () => {
  const digest = "1".repeat(64);
  const asset = EXPECTED_NATIVE_HELPER_ASSETS[0].asset;
  assert.throws(
    () => parseChecksumManifest(`${digest}  ${asset}\n${digest}  ${asset}\n`),
    /duplicate SHA256SUMS entry/,
  );
  for (const unsafeName of [`../${asset}`, `nested/${asset}`, `nested\\${asset}`]) {
    assert.throws(
      () => parseChecksumManifest(`${digest}  ${unsafeName}\n`),
      /invalid SHA256SUMS line/,
    );
  }
});

test("metadata builder rejects an unknown asset even with six entries", () => {
  const checksums = validChecksums();
  checksums.delete(EXPECTED_NATIVE_HELPER_ASSETS[0].asset);
  checksums.set("plantuml-export-unknown", "9".repeat(64));
  assert.throws(
    () => buildNativeHelperMetadata("v0.1.0-rc.1", checksums),
    /exactly the six expected native helper assets/,
  );
});

test("metadata builder pins exact tag URLs and validates every target", () => {
  const metadata = buildNativeHelperMetadata("v0.1.0-rc.1", validChecksums());

  assert.doesNotThrow(() => validateNativeHelperMetadata(metadata));
  assert.equal(metadata.status, "published");
  assert.equal(metadata.artifacts.length, 6);
  for (const artifact of metadata.artifacts) {
    assert.equal(
      artifact.url,
      `https://github.com/dahuangggg/plantuml-export/releases/download/v0.1.0-rc.1/${artifact.asset}`,
    );
  }
});

test("metadata generator refuses an incomplete checksum manifest", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "plantuml-export-metadata-"));
  const checksums = path.join(directory, "SHA256SUMS");
  const output = path.join(directory, "native-helper-release.json");
  fs.writeFileSync(checksums, `${"1".repeat(64)}  ${EXPECTED_NATIVE_HELPER_ASSETS[0].asset}\n`);

  assert.throws(
    () =>
      execFileSync(
        process.execPath,
        [
          "scripts/generate-native-helper-metadata.mjs",
          "--tag",
          "v0.1.0-rc.1",
          "--checksums",
          checksums,
          "--output",
          output,
        ],
        { cwd: root, stdio: "pipe" },
      ),
    /exactly the six expected native helper assets/,
  );
  assert.equal(fs.existsSync(output), false);
});

test("metadata generator atomically replaces unpublished metadata", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "plantuml-export-metadata-"));
  const checksums = path.join(directory, "SHA256SUMS");
  const output = path.join(directory, "native-helper-release.json");
  fs.writeFileSync(
    checksums,
    EXPECTED_NATIVE_HELPER_ASSETS.map(
      ({ asset }, index) => `${(index + 1).toString(16).padStart(64, "0")}  ${asset}`,
    ).join("\n") + "\n",
  );
  fs.writeFileSync(output, "{\"status\":\"unpublished\"}\n");

  execFileSync(
    process.execPath,
    [
      "scripts/generate-native-helper-metadata.mjs",
      "--tag",
      "v0.1.0-rc.1",
      "--checksums",
      checksums,
      "--output",
      output,
    ],
    { cwd: root, stdio: "pipe" },
  );

  const generated = JSON.parse(fs.readFileSync(output, "utf8"));
  assert.equal(generated.status, "published");
  assert.equal(generated.artifacts.length, 6);
  assert.deepEqual(
    fs.readdirSync(directory).sort(),
    ["SHA256SUMS", "native-helper-release.json"],
  );
});

function validChecksums() {
  return new Map(
    EXPECTED_NATIVE_HELPER_ASSETS.map(({ asset }, index) => [
      asset,
      (index + 1).toString(16).padStart(64, "0"),
    ]),
  );
}

function assertCheckedInMetadata(metadata) {
  assert.doesNotThrow(() => validateNativeHelperMetadata(metadata));
  if (metadata.status === "unpublished") {
    assert.deepEqual(metadata, {
      schemaVersion: 1,
      repository: "dahuangggg/plantuml-export",
      status: "unpublished",
      releaseTag: null,
      artifacts: [],
    });
    return;
  }

  assert.equal(metadata.status, "published");
  assert.match(metadata.releaseTag, /^v0\.1\.0(?:-rc\.[1-9][0-9]*)?$/);
  assert.equal(metadata.artifacts.length, EXPECTED_NATIVE_HELPER_ASSETS.length);
}
