#!/usr/bin/env node

import fs from "node:fs";
import { spawnSync } from "node:child_process";

import {
  buildRendererCommand,
  classifyRendererFailure,
  ensureRendererDownloads,
  listFreshOutputs,
  normalizeOutDir,
  parseArgs,
  requiredRendererDownloads,
  resolveInputs,
  resolveRenderer,
} from "../src/export-command.mjs";

try {
  const options = parseArgs(process.argv.slice(2));
  const outDir = normalizeOutDir(options.outDir);
  fs.mkdirSync(outDir, { recursive: true });

  const renderer = resolveRenderer({
    autoDownloadJar: options.autoDownloadJar,
    jar: options.jar,
    java: options.java,
    plantuml: options.plantuml,
    plantumlVersion: options.plantumlVersion,
    renderer: options.renderer,
    serverUrl: options.serverUrl,
  });

  await ensureRendererDownloads({
    downloads: requiredRendererDownloads({
      format: options.format,
      renderer,
    }),
    log: (message) => console.error(message),
  });

  const startedAt = Date.now();
  const inputs = resolveInputs({
    input: options.input,
    workspace: options.workspace,
  });

  if (inputs.length === 0) {
    throw new Error(`No PlantUML files found for ${options.input}`);
  }

  for (const input of inputs) {
    const { command, args } = buildRendererCommand({
      ...options,
      input,
      outDir,
      renderer,
    });

    const result = spawnSync(command, args, {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });

    if (result.stdout) {
      process.stdout.write(result.stdout);
    }
    if (result.stderr) {
      process.stderr.write(result.stderr);
    }

    if (result.error) {
      throw new Error(
        classifyRendererFailure({
          command,
          error: result.error,
          format: options.format,
        }),
      );
    }

    if (result.status !== 0) {
      throw new Error(
        classifyRendererFailure({
          command,
          format: options.format,
          output: `${result.stdout ?? ""}\n${result.stderr ?? ""}`,
          status: result.status,
        }),
      );
    }
  }

  const outputs = listFreshOutputs({
    format: options.format,
    outDir,
    since: startedAt,
  });

  if (outputs.length === 0) {
    throw new Error(
      `renderer reported success but no .${options.format} file was written to ${outDir}`,
    );
  }
} catch (error) {
  console.error(`PlantUML export failed: ${error.message}`);
  process.exit(1);
}
