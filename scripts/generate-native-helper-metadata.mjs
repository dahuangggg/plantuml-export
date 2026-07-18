#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

import {
  buildNativeHelperMetadata,
  parseChecksumManifest,
} from "./native-helper-metadata.mjs";

const options = parseArguments(process.argv.slice(2));
const checksums = parseChecksumManifest(fs.readFileSync(options.checksums, "utf8"));
const metadata = buildNativeHelperMetadata(options.tag, checksums);
const output = path.resolve(options.output);
const temporary = `${output}.tmp-${process.pid}`;

fs.mkdirSync(path.dirname(output), { recursive: true });
try {
  fs.writeFileSync(temporary, `${JSON.stringify(metadata, null, 2)}\n`, { flag: "wx" });
  replaceAtomically(temporary, output);
} catch (error) {
  fs.rmSync(temporary, { force: true });
  throw error;
}

function replaceAtomically(temporaryPath, outputPath) {
  const backup = `${outputPath}.previous-${process.pid}`;
  fs.rmSync(backup, { force: true });
  let hadOutput = false;
  try {
    fs.renameSync(outputPath, backup);
    hadOutput = true;
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }

  try {
    fs.renameSync(temporaryPath, outputPath);
  } catch (error) {
    if (hadOutput) fs.renameSync(backup, outputPath);
    throw error;
  }
  if (hadOutput) fs.rmSync(backup, { force: true });
}

function parseArguments(args) {
  const options = {};
  for (let index = 0; index < args.length; index += 2) {
    const name = args[index];
    const value = args[index + 1];
    if (!value || !["--tag", "--checksums", "--output"].includes(name)) {
      throw new Error(
        "usage: generate-native-helper-metadata --tag TAG --checksums SHA256SUMS --output FILE",
      );
    }
    if (options[name.slice(2)] !== undefined) {
      throw new Error(`duplicate argument ${name}`);
    }
    options[name.slice(2)] = value;
  }
  if (!options.tag || !options.checksums || !options.output) {
    throw new Error(
      "usage: generate-native-helper-metadata --tag TAG --checksums SHA256SUMS --output FILE",
    );
  }
  return options;
}
