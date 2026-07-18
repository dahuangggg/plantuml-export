#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);

const forbiddenTrackedPaths = [
  "extension.wasm",
  "grammars",
  "node_modules",
  "out",
  "target",
  "tmp",
  "vendor",
  "vendor/plantuml.jar",
];

const errors = [];
const trackedFiles = readTrackedFiles();

if (trackedFiles) {
  for (const relativePath of forbiddenTrackedPaths) {
    if (
      trackedFiles.some(
        (file) => file === relativePath || file.startsWith(`${relativePath}/`),
      )
    ) {
      errors.push(`Generated or local-only artifact is tracked: ${relativePath}`);
    }
  }

  for (const file of trackedFiles) {
    if (file.endsWith(".jar")) {
      errors.push(`Renderer jar must not be bundled in the extension source: ${file}`);
    }
  }
} else {
  for (const { relativePath, kind } of listPackageEntries(pathFromUrl(root))) {
    const forbiddenPath = forbiddenPathFor(relativePath);
    if (forbiddenPath) {
      errors.push(`Generated or local-only artifact is tracked: ${forbiddenPath}`);
      continue;
    }

    if (kind === "symlink") {
      errors.push(`Symbolic links must not be bundled in the extension source: ${relativePath}`);
      continue;
    }
    if (kind === "special") {
      errors.push(`Special files must not be bundled in the extension source: ${relativePath}`);
      continue;
    }
    if (relativePath.endsWith(".jar")) {
      errors.push(`Renderer jar must not be bundled in the extension source: ${relativePath}`);
    }
  }
}

if (errors.length > 0) {
  console.error("Release package check failed:");
  for (const error of errors) {
    console.error(`- ${error}`);
  }
  process.exit(1);
}

console.log("Release package check passed");

function listPackageEntries(directoryPath) {
  const entries = [];

  visit(directoryPath);
  return entries;

  function visit(currentPath) {
    for (const entry of fs.readdirSync(currentPath, { withFileTypes: true })) {
      const entryPath = path.join(currentPath, entry.name);
      const relativePath = normalizeRelativePath(
        path.relative(directoryPath, entryPath),
      );

      if (relativePath === ".git" || relativePath.startsWith(".git/")) {
        continue;
      }

      if (forbiddenPathFor(relativePath)) {
        entries.push({ relativePath, kind: entryKind(entry) });
        continue;
      }

      if (entry.isDirectory()) {
        visit(entryPath);
      } else {
        entries.push({ relativePath, kind: entryKind(entry) });
      }
    }
  }
}

function entryKind(entry) {
  if (entry.isFile()) return "file";
  if (entry.isDirectory()) return "directory";
  if (entry.isSymbolicLink()) return "symlink";
  return "special";
}

function forbiddenPathFor(relativePath) {
  const normalizedPath = normalizeRelativePath(relativePath);
  return forbiddenTrackedPaths.find(
    (forbiddenPath) =>
      normalizedPath === forbiddenPath ||
      normalizedPath.startsWith(`${forbiddenPath}/`),
  );
}

function normalizeRelativePath(relativePath) {
  return relativePath.split(path.sep).join("/");
}

function readTrackedFiles() {
  try {
    return execFileSync("git", ["ls-files"], {
      cwd: pathFromUrl(root),
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    })
      .split(/\r?\n/)
      .filter(Boolean);
  } catch {
    return undefined;
  }
}

function pathFromUrl(url) {
  return fileURLToPath(url);
}
