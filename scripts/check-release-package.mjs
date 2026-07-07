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
  for (const filePath of listFiles(root)) {
    const relativePath = path.relative(pathFromUrl(root), filePath);
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

function listFiles(directoryUrl) {
  const files = [];

  visit(directoryUrl);
  return files;

  function visit(currentUrl) {
    for (const entry of fs.readdirSync(currentUrl, { withFileTypes: true })) {
      if (
        entry.name === ".git" ||
        forbiddenTrackedPaths.includes(entry.name)
      ) {
        continue;
      }

      const entryUrl = new URL(`${entry.name}${entry.isDirectory() ? "/" : ""}`, currentUrl);
      if (entry.isDirectory()) {
        visit(entryUrl);
      } else if (entry.isFile()) {
        files.push(entryUrl.pathname);
      }
    }
  }
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
